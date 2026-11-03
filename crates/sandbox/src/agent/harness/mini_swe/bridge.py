"""Moatless JSONL transport for the pinned mini-swe-agent Python library."""

import json
import os
import queue
import signal
import subprocess
import sys
import threading
import uuid
from pathlib import Path

TRANSPORT = sys.stdout
sys.stdout = sys.stderr
os.environ["MSWEA_SILENT_STARTUP"] = "1"
os.environ["LITELLM_LOCAL_MODEL_COST_MAP"] = "True"
os.environ["MSWEA_MODEL_RETRY_STOP_AFTER_ATTEMPT"] = "3"
from minisweagent.agents.default import DefaultAgent
from minisweagent.environments.local import LocalEnvironment
from minisweagent.exceptions import FormatError, InterruptAgentFlow, Submitted
from minisweagent.models.litellm_model import LitellmModel
from minisweagent.models.litellm_response_model import LitellmResponseModel


def emit(kind, **data):
    print(json.dumps({"type": kind, **data}), file=TRANSPORT, flush=True)


class Environment(LocalEnvironment):
    def __init__(self, **kwargs):
        super().__init__(**kwargs)
        self.child = None

    def stop(self):
        if self.child is not None:
            try:
                os.killpg(self.child.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            self.child.wait()

    def execute(self, action, cwd="", *, timeout=None):
        emit("activity", detail="tool_start")
        self.child = subprocess.Popen(
            action["command"], shell=True, cwd=cwd or self.config.cwd,
            env=os.environ | self.config.env, start_new_session=True,
            stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True,
            encoding="utf-8", errors="replace",
        )
        try:
            output, _ = self.child.communicate(timeout=timeout or self.config.timeout)
            result = {"output": output, "returncode": self.child.returncode,
                      "exception_info": ""}
        except subprocess.TimeoutExpired:
            self.stop()
            output, _ = self.child.communicate()
            result = {"output": output, "returncode": -1,
                      "exception_info": "Command timed out"}
        finally:
            self.stop()
            self.child = None
        self._check_finished(result)
        return result


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
            message = {k: v for k, v in message.items() if k in ("role", "content", "extra")}
        result.append(message)
    return result


def main():
    request = json.loads(sys.stdin.readline())
    journal = Path(request["journal"])
    journal.parent.mkdir(parents=True, exist_ok=True)
    provider, name = request["model"].split("/", 1)
    model_type = LitellmModel if provider == "anthropic" else LitellmResponseModel
    model = model_type(
        model_name=request["model"], cost_tracking="ignore_errors",
        model_kwargs={"api_base": request["api_base"],
                      "api_key": Path(request["token_path"]).read_text().strip(),
                      "extra_headers": {"X-Moatless-Auth-Mode": "api-key-or-hub"},
                      "timeout": 120, "num_retries": 0},
    )
    model.abort_exceptions = [*model.abort_exceptions, SystemExit]
    env = Environment(cwd=request["cwd"], timeout=120)

    def cancel(signum, frame):
        env.stop()
        raise SystemExit(130)

    signal.signal(signal.SIGTERM, cancel)
    signal.signal(signal.SIGINT, cancel)
    agent = Agent(model, env, journal=journal, model_name=request["model"],
                  system_template="", instance_template="", step_limit=100, cost_limit=0)
    if request["resume"]:
        records = [json.loads(line) for line in journal.read_text().splitlines()]
        agent.messages = [r["message"] for r in records if r.get("type") == "message"
                          and r["message"].get("role") not in ("exit", "system")]
        agent.messages = model_messages(agent.messages, provider)
        agent.repair()
    agent.add_messages(model.format_message(role="system", content=request["system_prompt"]))
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
                agent.add_messages(*error.messages)
                agent.n_consecutive_format_errors += 1
                if agent.n_consecutive_format_errors >= 3:
                    emit("finished", exit_status="RepeatedFormatError")
                    return
            except InterruptAgentFlow as error:
                if isinstance(error, Submitted) and not inputs.empty():
                    continue
                agent.add_messages(*error.messages)
            if agent.messages[-1].get("role") == "exit":
                emit("finished", **agent.messages[-1].get("extra", {}))
                return
    except Exception as error:
        emit("finished", exit_status=type(error).__name__)
    finally:
        env.stop()


if __name__ == "__main__":
    main()
