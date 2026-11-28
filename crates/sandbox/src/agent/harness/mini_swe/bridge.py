"""Moatless JSONL transport for the pinned mini-swe-agent Python library."""

import json
import os
import queue
import signal
import subprocess
import sys
import threading
import traceback
import uuid
from pathlib import Path

TRANSPORT = sys.stdout
sys.stdout = sys.stderr
os.environ["MSWEA_SILENT_STARTUP"] = "1"
os.environ["LITELLM_LOCAL_MODEL_COST_MAP"] = "True"
os.environ["MSWEA_MODEL_RETRY_STOP_AFTER_ATTEMPT"] = "3"
import litellm
import yaml
from minisweagent.agents.default import DefaultAgent
from minisweagent.environments.local import LocalEnvironment, LocalEnvironmentConfig
from minisweagent.exceptions import FormatError, InterruptAgentFlow, LimitsExceeded, Submitted
from minisweagent.models.litellm_model import LitellmModel
from minisweagent.models.litellm_response_model import LitellmResponseModel

ERROR_LIMIT = 2000
"""Characters of an exception's text that reach the Turn's error."""


def emit(kind, **data):
    print(json.dumps({"type": kind, **data}), file=TRANSPORT, flush=True)


def load_config(text=None):
    """Moatless's `mini.yaml`: `text` from the request, else the file beside this one."""
    if text is None:
        text = Path(__file__).with_name("mini.yaml").read_text()
    return yaml.safe_load(text)


def elide(text, limit, note):
    """`text` cut to its head and tail around `note` when longer than `limit`."""
    if len(text) <= limit:
        return text
    half = limit // 2
    return f"{text[:half]}\n[... {len(text) - 2 * half} characters elided; {note} ...]\n{text[-half:]}"


class EnvironmentConfig(LocalEnvironmentConfig):
    max_timeout: int = 600
    output_limit: int = 10000
    output_dir: str = ""


class Environment(LocalEnvironment):
    def __init__(self, **kwargs):
        super().__init__(config_class=EnvironmentConfig, **kwargs)
        self.child = None

    def get_template_vars(self, **kwargs):
        return {**super().get_template_vars(**kwargs), **self.config.model_dump()}

    def stop(self):
        if self.child is not None:
            try:
                os.killpg(self.child.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            self.child.wait()

    def cap(self, output):
        """`output` within `output_limit`, its full text spilled to `output_dir`.

        Do not let an uncapped output through: it is journaled twice and replayed
        on every resume, so one large output overflows every later model call.
        """
        if len(output) <= self.config.output_limit:
            return output
        note = "the full output was not saved"
        if self.config.output_dir:
            try:
                path = Path(self.config.output_dir) / f"{uuid.uuid4()}.txt"
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text(output, encoding="utf-8")
                note = f"full output saved to {path}"
            except OSError as error:
                note = f"the full output could not be saved: {error}"
        return elide(output, self.config.output_limit, note)

    def execute(self, action, cwd="", *, timeout=None):
        emit("activity", detail="tool_start")
        limit = min(timeout or action.get("timeout") or self.config.timeout, self.config.max_timeout)
        self.child = subprocess.Popen(
            action["command"], shell=True, cwd=cwd or self.config.cwd,
            env=os.environ | self.config.env, start_new_session=True,
            stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True,
            encoding="utf-8", errors="replace",
        )
        try:
            output, _ = self.child.communicate(timeout=limit)
            result = {"output": output, "returncode": self.child.returncode,
                      "exception_info": ""}
        except subprocess.TimeoutExpired:
            self.stop()
            output, _ = self.child.communicate()
            result = {"output": output, "returncode": -1,
                      "exception_info": f"Command timed out after {limit} seconds"}
        finally:
            self.stop()
            self.child = None
        result["output"] = self.cap(result["output"])
        self._check_finished(result)
        return result


def bash_tool(env, responses):
    """The `bash` tool, with the optional `timeout` upstream's schema lacks."""
    parameters = {
        "type": "object",
        "properties": {
            "command": {"type": "string", "description": "The bash command to execute"},
            "timeout": {"type": "integer", "description":
                        f"Seconds before the command is killed. Default {env.config.timeout}, "
                        f"maximum {env.config.max_timeout}."},
        },
        "required": ["command"],
    }
    if responses:
        return {"type": "function", "name": "bash", "description": "Execute a bash command",
                "parameters": parameters}
    return {"type": "function", "function": {"name": "bash", "description": "Execute a bash command",
                                             "parameters": parameters}}


def with_timeouts(actions, arguments):
    """`actions` with each call's integer `timeout` argument, keyed by call id."""
    for action in actions:
        try:
            timeout = json.loads(arguments.get(action.get("tool_call_id")) or "{}").get("timeout")
        except (ValueError, AttributeError):
            timeout = None
        if isinstance(timeout, int) and timeout > 0:
            action["timeout"] = timeout
    return actions


class ChatModel(LitellmModel):
    def __init__(self, *, tool, **kwargs):
        super().__init__(**kwargs)
        self.tool = tool

    def _query(self, messages, **kwargs):
        return litellm.completion(model=self.config.model_name, messages=messages,
                                  tools=[self.tool], **(self.config.model_kwargs | kwargs))

    def _parse_actions(self, response):
        calls = response.choices[0].message.tool_calls or []
        return with_timeouts(super()._parse_actions(response),
                             {c.id: c.function.arguments for c in calls})


class ResponsesModel(LitellmResponseModel):
    def __init__(self, *, tool, **kwargs):
        super().__init__(**kwargs)
        self.tool = tool

    def _query(self, messages, **kwargs):
        return litellm.responses(model=self.config.model_name, input=messages,
                                 tools=[self.tool], **(self.config.model_kwargs | kwargs))

    def _parse_actions(self, response):
        arguments = {}
        for item in getattr(response, "output", None) or []:
            item = item if isinstance(item, dict) else item.model_dump()
            if item.get("type") == "function_call":
                arguments[item.get("call_id") or item.get("id")] = item.get("arguments")
        return with_timeouts(super()._parse_actions(response), arguments)


def response_text(response):
    """The text of a chat or Responses API dump, as a FormatError keeps it."""
    if not isinstance(response, dict):
        return ""
    if response.get("choices"):
        return response["choices"][0].get("message", {}).get("content") or ""
    return "\n".join(block.get("text", "") for item in response.get("output") or []
                     if item.get("type") == "message" for block in item.get("content") or [])


def assistant_text(text, provider):
    """An assistant message holding only `text`, in `provider`'s history shape."""
    if provider == "openai":
        return {"type": "message", "role": "assistant",
                "content": [{"type": "output_text", "text": text}]}
    return {"role": "assistant", "content": text}


def exit_message(status, content, **extra):
    return {"role": "exit", "content": content,
            "extra": {"exit_status": status, "submission": "", **extra}}


class Agent(DefaultAgent):
    def __init__(self, *args, journal, model_name, **kwargs):
        super().__init__(*args, **kwargs)
        self.journal = journal
        self.model_name = model_name
        self.streaming = False

    def add_messages(self, *messages):
        records = [{"type": "message", "id": str(uuid.uuid4()),
                    "model": self.model_name, "message": m} for m in messages]
        with self.journal.open("a") as stream:
            for record in records:
                stream.write(json.dumps(record) + "\n")
            stream.flush()
            os.fsync(stream.fileno())
        result = super().add_messages(*messages)
        if self.streaming:
            for record in records:
                emit("message", record=record)
        return result

    def execute_actions(self, message):
        outputs = []
        submitted = None
        for action in message.get("extra", {}).get("actions", []):
            if submitted is not None:
                outputs.append({"output": "Not executed: task submitted", "returncode": -1,
                                "exception_info": ""})
                continue
            try:
                outputs.append(self.env.execute(action))
            except Submitted as error:
                submitted = error
                outputs.append({"output": error.messages[0]["content"], "returncode": 0,
                                "exception_info": ""})
        result = self.add_messages(*self.model.format_observation_messages(
            message, outputs, self.get_template_vars()))
        if submitted is not None:
            raise submitted
        return result

    def repair(self):
        pending = {}
        for message in self.messages:
            for action in message.get("extra", {}).get("actions", []):
                pending[action.get("tool_call_id", action.get("call_id", action.get("id")))] = message
            if message.get("role") == "tool":
                pending.pop(message.get("tool_call_id"), None)
            if message.get("type") == "function_call_output":
                pending.pop(message.get("call_id"), None)
        for message in {id(m): m for m in pending.values()}.values():
            actions = message.get("extra", {}).get("actions", [])
            outputs = [{"output": "Interrupted; inspect workspace before retrying", "returncode": -1,
                        "exception_info": "Interrupted"} for _ in actions]
            for observation in self.model.format_observation_messages(message, outputs, self.get_template_vars()):
                call_id = observation.get("tool_call_id", observation.get("call_id"))
                if call_id in pending:
                    self.add_messages(observation)


def cap_history(messages, limit):
    """`messages` with each tool result cut to `limit`, for a journal written
    before `Environment.cap` existed: replayed whole, one oversized result
    overflows every model call the session makes."""
    note = "cut when the session was restored"
    for message in messages:
        for key in ("content", "output"):
            if (message.get("role") == "tool" or message.get("type") == "function_call_output") \
                    and isinstance(message.get(key), str):
                message[key] = elide(message[key], limit, note)
        extra = message.get("extra") or {}
        if isinstance(extra.get("raw_output"), str):
            extra["raw_output"] = elide(extra["raw_output"], limit, note)
    return messages


def model_messages(messages, provider):
    result = []
    for original in messages:
        message = dict(original)
        if provider == "anthropic" and message.get("object") == "response":
            text = []
            calls = []
            for item in message.get("output", []):
                if item.get("type") == "message":
                    text.extend(b.get("text", "") for b in item.get("content", []))
                elif item.get("type") == "function_call":
                    calls.append({"id": item["call_id"], "type": "function",
                                  "function": {"name": item["name"], "arguments": item["arguments"]}})
            message = {"role": "assistant", "content": "\n".join(text),
                       "extra": message.get("extra", {})}
            if calls:
                message["tool_calls"] = calls
        elif provider == "anthropic" and message.get("type") == "function_call_output":
            message = {"role": "tool", "tool_call_id": message["call_id"],
                       "content": message["output"], "extra": message.get("extra", {})}
        elif provider == "openai" and message.get("role") == "assistant" and message.get("tool_calls"):
            output = []
            if message.get("content"):
                output.append({"type": "message", "role": "assistant", "content": [
                    {"type": "output_text", "text": message["content"]}]})
            output.extend({"type": "function_call", "call_id": c["id"],
                           "name": c["function"]["name"], "arguments": c["function"]["arguments"]}
                          for c in message["tool_calls"])
            message = {"object": "response", "output": output, "extra": message.get("extra", {})}
        elif provider == "openai" and message.get("role") == "tool":
            message = {"type": "function_call_output", "call_id": message["tool_call_id"],
                       "output": message.get("content", ""), "extra": message.get("extra", {})}
        if provider == "anthropic" and isinstance(message.get("content"), list):
            message["content"] = [{**b, "type": "text"} if b.get("type") in ("input_text", "output_text")
                                  else b for b in message["content"]]
        if provider == "openai" and message.get("role") == "assistant":
            message = {k: v for k, v in message.items() if k in ("role", "content", "extra", "type")}
        result.append(message)
    return result


def build(request):
    """The agent `request` names, configured from Moatless's `mini.yaml`."""
    config = load_config(request.get("config"))
    journal = Path(request["journal"])
    journal.parent.mkdir(parents=True, exist_ok=True)
    provider = request["model"].split("/", 1)[0]
    env = Environment(**config.get("environment", {}), cwd=request["cwd"],
                      output_dir=request.get("output_dir", ""))
    responses = provider != "anthropic"
    model_config = dict(config.get("model", {}))
    model_kwargs = model_config.pop("model_kwargs", {})
    model_type = ResponsesModel if responses else ChatModel
    model = model_type(
        **model_config, tool=bash_tool(env, responses), model_name=request["model"],
        model_kwargs={**model_kwargs,
                      "api_base": request["api_base"],
                      "api_key": Path(request["token_path"]).read_text().strip(),
                      "extra_headers": {"X-Moatless-Auth-Mode": "api-key-or-hub"}},
    )
    model.abort_exceptions = [*model.abort_exceptions, SystemExit]
    agent = Agent(model, env, journal=journal, model_name=request["model"], **config.get("agent", {}))
    agent.extra_template_vars |= {"moatless_system_prompt": request["system_prompt"],
                                  "skills": request.get("skills", "")}
    return agent, provider


def main():
    request = json.loads(sys.stdin.readline())
    agent, provider = build(request)
    env, model = agent.env, agent.model

    def cancel(signum, frame):
        env.stop()
        raise SystemExit(130)

    signal.signal(signal.SIGTERM, cancel)
    signal.signal(signal.SIGINT, cancel)
    if request["resume"]:
        records = [json.loads(line) for line in agent.journal.read_text().splitlines()]
        agent.messages = [r["message"] for r in records if r.get("type") == "message"
                          and r["message"].get("role") not in ("exit", "system")]
        agent.messages = cap_history(model_messages(agent.messages, provider),
                                     env.config.output_limit)
        agent.repair()
    agent.add_messages(model.format_message(
        role="system", content=agent._render_template(agent.config.system_template)))
    agent.messages.insert(0, agent.messages.pop())
    inputs = queue.Queue(maxsize=16)

    def read_inputs():
        for line in sys.stdin:
            try:
                inputs.put(json.loads(line))
            except ValueError:
                os.kill(os.getpid(), signal.SIGTERM)
                return
        os.kill(os.getpid(), signal.SIGTERM)

    threading.Thread(target=read_inputs, daemon=True).start()
    emit("ready", session=request["session"])
    agent.streaming = True
    try:
        first = True
        while True:
            if first:
                items = [inputs.get()]
                first = False
            else:
                items = []
            while True:
                try:
                    items.append(inputs.get_nowait())
                except queue.Empty:
                    break
            for item in items:
                agent.add_messages(model.format_message(role="user", content=item["text"]))
                emit("input_accepted", id=item["id"])
            try:
                agent.step()
                agent.n_consecutive_format_errors = 0
            except FormatError as error:
                text = response_text(error.messages[0].get("extra", {}).get("response"))
                if text.strip():
                    agent.add_messages(assistant_text(text, provider))
                agent.add_messages(*error.messages)
                agent.n_consecutive_format_errors += 1
                if 0 < agent.config.max_consecutive_format_errors <= agent.n_consecutive_format_errors:
                    agent.add_messages(exit_message(
                        "RepeatedFormatError",
                        f"Stopped: {agent.n_consecutive_format_errors} responses in a row made no valid tool call."))
            except LimitsExceeded:
                agent.add_messages(exit_message(
                    "LimitsExceeded",
                    f"Paused after {agent.n_calls} model calls, this turn's step limit. "
                    "Send a message to continue."))
            except InterruptAgentFlow as error:
                if isinstance(error, Submitted) and not inputs.empty():
                    continue
                agent.add_messages(*error.messages)
            if agent.messages[-1].get("role") == "exit":
                emit("finished", **agent.messages[-1].get("extra", {}))
                return
    except Exception as error:
        traceback.print_exc()
        detail = f"{type(error).__name__}: {error}"[:ERROR_LIMIT]
        try:
            agent.add_messages(exit_message(type(error).__name__, detail))
        except Exception:
            traceback.print_exc()
        emit("finished", exit_status=type(error).__name__, error=detail)
    finally:
        env.stop()


if __name__ == "__main__":
    main()
