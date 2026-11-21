#!/usr/bin/env python3
"""A stand-in ACP agent for the harness tests.

Logs every request as a JSON line to $FAKE_ACP_LOG and keeps each session it
created as a file where agy keeps its conversations. Like agy, it accepts
`session/load` of an unknown id and fails the prompt that follows.
"""
import json, os, queue, sys, threading, time

gdir = sys.argv[sys.argv.index("--gemini-dir") + 1]
conversations = os.path.join(gdir, "antigravity-cli", "conversations")
os.makedirs(conversations, exist_ok=True)
log = open(os.environ["FAKE_ACP_LOG"], "a")
inbox = queue.Queue()
lock = threading.Lock()


def send(msg):
    with lock:
        sys.stdout.write(json.dumps(msg) + "\n")
        sys.stdout.flush()


def update(sid, u):
    send({"jsonrpc": "2.0", "method": "session/update", "params": {"sessionId": sid, "update": u}})


def reader():
    for line in sys.stdin:
        if line.strip():
            inbox.put(json.loads(line))
    inbox.put(None)


threading.Thread(target=reader, daemon=True).start()
cancelled = threading.Event()
usage = {"inputTokens": 10, "outputTokens": 5, "cachedReadTokens": 100, "cachedWriteTokens": 0, "thoughtTokens": 2, "totalTokens": 117}


def prompt(msg):
    sid = msg["params"]["sessionId"]
    text = msg["params"]["prompt"][0]["text"]
    if not os.path.exists(os.path.join(conversations, sid + ".db")):
        send({"jsonrpc": "2.0", "id": msg["id"], "error": {"code": -32603, "message": "trajectory not found: " + sid}})
        return
    if "SLOW" in text:
        update(sid, {"sessionUpdate": "tool_call", "toolCallId": "slow", "title": "sleep", "kind": "execute", "status": "in_progress", "rawInput": {"command": "sleep 60"}})
        cancelled.wait(30)
        send({"jsonrpc": "2.0", "id": msg["id"], "result": {"stopReason": "cancelled"}})
        return
    if "BGTOOL" in text:
        update(sid, {"sessionUpdate": "tool_call", "toolCallId": "bg", "title": "sleep", "kind": "execute", "status": "in_progress", "rawInput": {"command": "sleep 1"}})
        send({"jsonrpc": "2.0", "id": msg["id"], "result": {"stopReason": "end_turn", "usage": usage}})
        time.sleep(0.5)
        update(sid, {"sessionUpdate": "tool_call_update", "toolCallId": "bg", "status": "completed",
                     "content": [{"type": "content", "content": {"type": "text", "text": "slept"}}]})
        update(sid, {"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "bg done"}})
        return
    if "PERMISSION" in text:
        send({"jsonrpc": "2.0", "id": "perm-1", "method": "session/request_permission", "params": {"sessionId": sid, "options": [
            {"optionId": "reject", "kind": "reject_once"}, {"optionId": "yes", "kind": "allow_once"}]}})
    update(sid, {"sessionUpdate": "agent_thought_chunk", "content": {"type": "text", "text": "thinking"}})
    update(sid, {"sessionUpdate": "tool_call", "toolCallId": "call-1", "title": "list files", "kind": "execute", "status": "in_progress", "rawInput": {"command": "ls"}})
    update(sid, {"sessionUpdate": "tool_call_update", "toolCallId": "call-1", "status": "completed",
                 "content": [{"type": "content", "content": {"type": "text", "text": "a.txt"}}]})
    for chunk in ["echo: ", text[-40:]]:
        update(sid, {"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": chunk}})
    update(sid, {"sessionUpdate": "usage_update", "used": 1234, "size": 200000})
    send({"jsonrpc": "2.0", "id": msg["id"], "result": {"stopReason": "end_turn", "usage": usage}})
    if "BACKGROUND" in text:
        time.sleep(0.3)
        update(sid, {"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "woke up"}})


while True:
    msg = inbox.get()
    if msg is None:
        break
    method = msg.get("method")
    log.write(json.dumps(msg) + "\n")
    log.flush()
    if method is None:
        continue
    params = msg.get("params", {})
    if method == "initialize":
        result = {"protocolVersion": 1, "agentCapabilities": {"loadSession": True}}
    elif method == "session/new":
        sid = "s-%d" % time.time_ns()
        open(os.path.join(conversations, sid + ".db"), "w").close()
        result = {"sessionId": sid}
    elif method == "session/load":
        if os.path.exists(os.path.join(conversations, params["sessionId"] + ".db")):
            update(params["sessionId"], {"sessionUpdate": "user_message_chunk", "content": {"type": "text", "text": "replayed"}})
        result = None
    elif method == "session/set_config_option":
        if params["configId"] == "model" and params["value"] == "unknown-model":
            send({"jsonrpc": "2.0", "id": msg["id"], "error": {"code": -32602, "message": "unknown model"}})
            continue
        result = {"configOptions": []}
    elif method == "session/prompt":
        cancelled.clear()
        threading.Thread(target=prompt, args=(msg,), daemon=True).start()
        continue
    elif method == "session/cancel":
        cancelled.set()
        continue
    else:
        send({"jsonrpc": "2.0", "id": msg["id"], "error": {"code": -32601, "message": method}})
        continue
    send({"jsonrpc": "2.0", "id": msg["id"], "result": result})
