#!/usr/bin/env python3
"""A local stand-in for the OpenAI Responses API, for tests/codex/argv-check.sh (no network, no key).
Turn 1: if Codex offers the forge MCP tools (a `namespace` tool holding forge_get_knowledge), the mock
calls forge_get_knowledge. Turn 2 (a tool output came back): the final answer is that output.
Usage: mock_responses.py <port> <request-log>"""
import http.server
import json
import sys
import uuid

LOG = sys.argv[2]


class Handler(http.server.BaseHTTPRequestHandler):
    def log_message(self, *args):
        pass

    def do_GET(self):  # model list
        self.send_response(200)
        self.send_header("content-type", "application/json")
        self.end_headers()
        self.wfile.write(b'{"object":"list","data":[{"id":"mock-model","object":"model"}]}')

    def do_POST(self):
        body = json.loads(self.rfile.read(int(self.headers.get("content-length", 0))) or b"{}")
        with open(LOG, "a") as f:
            f.write(json.dumps({"path": self.path, "tools": [t.get("name") for t in body.get("tools", [])]}) + "\n")
        outputs = [i for i in body.get("input", []) if isinstance(i, dict) and i.get("type") == "function_call_output"]
        ns = next((t for t in body.get("tools", []) if t.get("type") == "namespace"
                   and any(x.get("name") == "forge_get_knowledge" for x in t.get("tools", []))), None)
        if outputs:
            text = "TOOL OUTPUT: " + json.dumps(outputs[-1].get("output"))
            item = {"type": "message", "role": "assistant", "id": "m2", "content": [{"type": "output_text", "text": text}]}
        elif ns:
            item = {"type": "function_call", "id": "fc1", "call_id": "call_" + uuid.uuid4().hex[:8],
                    "name": "forge_get_knowledge", "namespace": ns["name"], "arguments": "{}"}
        else:
            item = {"type": "message", "role": "assistant", "id": "m1", "content": [{"type": "output_text", "text": "NO FORGE TOOL OFFERED"}]}
        self.send_response(200)
        self.send_header("content-type", "text/event-stream")
        self.end_headers()
        rid = "resp_" + uuid.uuid4().hex[:8]
        usage = {"input_tokens": 1, "input_tokens_details": {"cached_tokens": 0}, "output_tokens": 1,
                 "output_tokens_details": {"reasoning_tokens": 0}, "total_tokens": 2}
        for kind, data in (("response.created", {"response": {"id": rid}}),
                           ("response.output_item.done", {"item": item, "output_index": 0}),
                           ("response.completed", {"response": {"id": rid, "output": [item], "usage": usage}})):
            self.wfile.write(f"event: {kind}\ndata: {json.dumps(dict(type=kind, **data))}\n\n".encode())


http.server.ThreadingHTTPServer(("127.0.0.1", int(sys.argv[1])), Handler).serve_forever()
