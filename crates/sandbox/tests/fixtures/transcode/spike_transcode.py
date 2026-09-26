#!/usr/bin/env python3
"""Phase 0 spike: transcode a recorded session into the other harness's format.

Writes the fixtures in this directory from the two recordings. Deterministic, so
rerunning it changes nothing unless the recordings or the rules change. Phase 2's
`SessionFormat::render` replaces it.

usage: transcode.py            # regenerate every *-from-*.jsonl here
"""
import json
import os
import uuid

HERE = os.path.dirname(os.path.abspath(__file__))
SESSION_ID = '{{SESSION_ID}}'
CWD = '{{CWD}}'
# Where the recordings ran; a transcoded path is rewritten to the target's cwd.
RECORDED_CWD = '/workspace'
TIMESTAMP = '2026-09-26T00:00:00.000Z'
NAMESPACE = uuid.UUID('6f1c7f0e-7d7b-4a53-9a39-6f4e3c1d2b10')
# Lines 0..N of each recording hold its first two turns, before the compaction.
TWO_TURN_LINES = {'claude': 39, 'codex': 38}
# Text the harness wrote into the user role itself: never shown, never transcoded.
CLAUDE_INJECTED = ('<command-name>', '<local-command-stdout>', '<local-command-caveat>',
                   '<system-reminder>', '<command-message>')
CODEX_INJECTED = ('<environment_context>', '<user_instructions>', '# AGENTS.md', '<turn_aborted>',
                  '<skills_instructions>')


# ── neutral items (what `read_line` + `transcode_items` will produce) ──────────

def claude_items(lines):
    items = []
    for line in lines:
        d = json.loads(line)
        if d.get('type') not in ('user', 'assistant') or d.get('isMeta') or d.get('isSidechain'):
            continue
        m = d['message']
        c = m.get('content')
        blocks = [{'type': 'text', 'text': c}] if isinstance(c, str) else c
        blocks = [b for b in blocks if b.get('type') in ('text', 'tool_use', 'tool_result')]
        if d.get('isCompactSummary'):
            # S5: transcode from the last compaction, its summary one user text item.
            items = [{'role': 'user', 'blocks': blocks}]
            continue
        if m['role'] == 'user' and isinstance(c, str) and c.lstrip().startswith(CLAUDE_INJECTED):
            continue
        if blocks:
            items.append({'role': m['role'], 'blocks': blocks})
    return items


def codex_items(lines):
    items = []
    for line in lines:
        d = json.loads(line)
        p = d.get('payload', {})
        if d['type'] == 'compacted':
            items = [{'role': 'user', 'blocks': [{'type': 'text', 'text': p['message']}]}]
            continue
        if d['type'] != 'response_item':
            continue
        t = p.get('type')
        if t == 'message':
            if p['role'] not in ('user', 'assistant'):
                continue
            text = ''.join(c.get('text', '') for c in p['content'])
            if p['role'] == 'user' and text.lstrip().startswith(CODEX_INJECTED):
                continue
            items.append({'role': p['role'], 'blocks': [{'type': 'text', 'text': text}]})
        elif t in ('function_call', 'custom_tool_call'):
            if t == 'function_call':
                inp = json.loads(p['arguments'])
            else:
                inp = {'input': p['input']}
            items.append({'role': 'assistant',
                          'blocks': [{'type': 'tool_use', 'id': p['call_id'], 'name': p['name'], 'input': inp}]})
        elif t in ('function_call_output', 'custom_tool_call_output'):
            items.append({'role': 'user',
                          'blocks': [{'type': 'tool_result', 'tool_use_id': p['call_id'], 'content': p['output']}]})
    return items


def result_text(content):
    if isinstance(content, list):
        return '\n'.join(c.get('text', '') for c in content if isinstance(c, dict))
    return content if isinstance(content, str) else json.dumps(content)


# ── ToolMap ──────────────────────────────────────────────────────────────────

def to_codex_call(block):
    """(item type, name, arguments or input) for a Claude call; None has no counterpart."""
    name, inp = block['name'], block['input']
    if name == 'Bash':
        return 'function_call', 'exec_command', json.dumps({'cmd': inp['command']})
    if name == 'Write':
        body = ''.join('+' + l + '\n' for l in inp['content'].splitlines())
        return 'custom_tool_call', 'apply_patch', \
            f"*** Begin Patch\n*** Add File: {inp['file_path']}\n{body}*** End Patch\n"
    if name == 'Edit':
        old = ''.join('-' + l + '\n' for l in inp['old_string'].splitlines())
        new = ''.join('+' + l + '\n' for l in inp['new_string'].splitlines())
        return 'custom_tool_call', 'apply_patch', \
            f"*** Begin Patch\n*** Update File: {inp['file_path']}\n@@\n{old}{new}*** End Patch\n"
    return None


def to_claude_call(block):
    """(name, input) for a Codex call; None has no counterpart."""
    name, inp = block['name'], block['input']
    if name in ('exec_command', 'shell', 'local_shell'):
        cmd = inp.get('cmd') or inp.get('command')
        if isinstance(cmd, list):
            cmd = cmd[-1] if cmd[:2] == ['bash', '-lc'] else ' '.join(cmd)
        return 'Bash', {'command': cmd}
    return None


def described(block, result):
    """The text fallback: one assistant text item standing for a call and its result."""
    return (f"[Tool call `{block['name']}` with input {json.dumps(block['input'])}, "
            f"made in this conversation by another agent harness. It returned:\n{result_text(result)}]")


def results_by_call(items):
    return {b['tool_use_id']: b for it in items for b in it['blocks'] if b['type'] == 'tool_result'}


# ── renderers (what `SessionFormat::render` will produce) ────────────────────

def render_codex(items):
    out = [{'timestamp': TIMESTAMP, 'type': 'session_meta', 'payload': {
        'id': SESSION_ID, 'timestamp': TIMESTAMP, 'cwd': CWD, 'originator': 'moatless_transcode',
        'cli_version': '0.156.1', 'source': 'exec'}}]
    results = results_by_call(items)
    described_calls, custom_calls = set(), set()

    def item(p):
        out.append({'timestamp': TIMESTAMP, 'type': 'response_item', 'payload': p})

    for it in items:
        for b in it['blocks']:
            if b['type'] == 'text':
                kind = 'input_text' if it['role'] == 'user' else 'output_text'
                item({'type': 'message', 'role': it['role'], 'content': [{'type': kind, 'text': b['text']}]})
            elif b['type'] == 'tool_use':
                mapped = to_codex_call(b)
                if mapped is None:
                    described_calls.add(b['id'])
                    text = described(b, results.get(b['id'], {}).get('content'))
                    item({'type': 'message', 'role': 'assistant', 'content': [{'type': 'output_text', 'text': text}]})
                    continue
                kind, name, payload = mapped
                if kind == 'function_call':
                    item({'type': 'function_call', 'name': name, 'arguments': payload, 'call_id': b['id']})
                else:
                    custom_calls.add(b['id'])
                    item({'type': 'custom_tool_call', 'status': 'completed', 'call_id': b['id'],
                          'name': name, 'input': payload})
            elif b['type'] == 'tool_result' and b['tool_use_id'] not in described_calls:
                kind = 'custom_tool_call_output' if b['tool_use_id'] in custom_calls else 'function_call_output'
                item({'type': kind, 'call_id': b['tool_use_id'], 'output': result_text(b.get('content'))})
    return out


def render_claude(items, model='claude-haiku-4-5-20251001'):
    out = []
    parent = None
    results = results_by_call(items)
    described_calls = set()

    def line(role, content):
        nonlocal parent
        u = str(uuid.uuid5(NAMESPACE, f'line:{len(out)}'))
        d = {'parentUuid': parent, 'isSidechain': False, 'type': role, 'uuid': u, 'timestamp': TIMESTAMP,
             'userType': 'external', 'cwd': CWD, 'sessionId': SESSION_ID, 'version': '2.1.281'}
        if role == 'user':
            d['message'] = {'role': 'user', 'content': content}
        else:
            d['message'] = {'id': f'msg_transcoded_{len(out)}', 'type': 'message', 'role': 'assistant',
                            'model': model, 'content': content, 'stop_reason': None, 'stop_sequence': None,
                            'usage': {'input_tokens': 0, 'output_tokens': 0}}
        out.append(d)
        parent = u

    for it in items:
        for b in it['blocks']:
            if b['type'] == 'text':
                line(it['role'], b['text'] if it['role'] == 'user' else [{'type': 'text', 'text': b['text']}])
            elif b['type'] == 'tool_use':
                mapped = to_claude_call(b)
                if mapped is None:
                    described_calls.add(b['id'])
                    text = described(b, results.get(b['id'], {}).get('content'))
                    line('assistant', [{'type': 'text', 'text': text}])
                    continue
                name, inp = mapped
                line('assistant', [{'type': 'tool_use', 'id': b['id'], 'name': name, 'input': inp}])
            elif b['type'] == 'tool_result' and b['tool_use_id'] not in described_calls:
                line('user', [{'type': 'tool_result', 'tool_use_id': b['tool_use_id'],
                               'content': result_text(b.get('content'))}])
    return out


def write(name, lines):
    with open(os.path.join(HERE, name), 'w') as f:
        for d in lines:
            f.write(json.dumps(d, separators=(',', ':'), ensure_ascii=False).replace(RECORDED_CWD, CWD) + '\n')


if __name__ == '__main__':
    recorded = {h: open(os.path.join(HERE, f'{h}.jsonl')).read().splitlines() for h in ('claude', 'codex')}
    two = {h: recorded[h][:TWO_TURN_LINES[h]] for h in recorded}
    write('codex-from-claude.jsonl', render_codex(claude_items(two['claude'])))
    write('codex-from-claude-compacted.jsonl', render_codex(claude_items(recorded['claude'])))
    write('claude-from-codex.jsonl', render_claude(codex_items(two['codex'])))
    write('claude-from-codex-compacted.jsonl', render_claude(codex_items(recorded['codex'])))
