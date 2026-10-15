#!/usr/bin/env python3
"""Runs inside the network-none container for tests/codex/argv-check.sh. For one adapter argv:
1. parse: `codex <argv> --help` exits 0, so codex-cli accepts every flag the adapter passes;
2. config: the argv's `-c` overrides load (`codex <overrides> mcp get forge`) and approve forge's tools;
3. mcp (only when the argv defines the forge MCP server): the exact argv runs against the local mock
   model, which calls forge_get_knowledge; the rollout must show that call `completed` and the answer
   must hold the marker captured into the project beforehand.
Usage: in_container.py '<json argv>'   Prints one line per step; exits 1 on any failure."""
import glob, json, os, subprocess, sys, time

argv = json.loads(sys.argv[1])
MARKER = "FORGE-ARGV-CHECK-7d41"
codex = "/opt/codex/bin/codex"
env = dict(os.environ, HOME="/home/run", CODEX_HOME="/home/run/.codex", BROWSER="/bin/false",
           OPENAI_API_KEY="mock-not-a-key")
os.makedirs(env["CODEX_HOME"], exist_ok=True)
failed = False


def step(name, ok, detail=""):
    global failed
    failed |= not ok
    print(f"  {'ok  ' if ok else 'FAIL'} {name}{': ' + detail if detail else ''}")


def run(args, **kw):
    return subprocess.run(args, env=env, capture_output=True, text=True, stdin=subprocess.DEVNULL, timeout=120, **kw)


ver = run([codex, "--version"]).stdout.strip()
step("codex version", ver == "codex-cli 0.162.1", ver)

r = run([codex, *argv, "--help"])
step("parse: codex accepts every flag", r.returncode == 0,
     "" if r.returncode == 0 else (r.stderr.strip().splitlines() or ["rc %d" % r.returncode])[0])

overrides = [argv[i + 1] for i, a in enumerate(argv[:-1]) if a == "-c"]
defines_server = any(o.startswith("mcp_servers.forge.command=") for o in overrides)
if overrides:
    cargs = [x for o in overrides for x in ("-c", o)]
    r = run([codex, *cargs, "mcp", "get", "forge"])
    step("config: overrides load", r.returncode == 0, "" if r.returncode == 0 else r.stderr.strip()[-200:])
    if any(o.startswith("mcp_servers.forge.default_tools_approval_mode=") for o in overrides):
        step("config: forge tools approved", "default_tools_approval_mode: approve" in r.stdout)

if defines_server:
    # The real forge binary stands in for the path the dump recorded (the test binary's own path).
    run_argv = [("mcp_servers.forge.command=\"/usr/local/bin/forge\""
                 if a.startswith("mcp_servers.forge.command=") else a) for a in argv]
    os.makedirs("/work", exist_ok=True)
    subprocess.run(["git", "init", "-q", "/work"], check=True)
    subprocess.run(["forge", "init"], cwd="/work", env=env, capture_output=True, check=True)
    msgs = [{"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2024-11-05", "capabilities": {}, "clientInfo": {"name": "argv-check", "version": "1"}}},
            {"jsonrpc": "2.0", "method": "notifications/initialized"},
            {"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": {"name": "forge_capture_knowledge", "arguments": {"title": "argv check", "content": f"Decision {MARKER}: argv check.", "category": "decisions"}}}]
    cap = subprocess.run(["forge", "mcp"], cwd="/work", env=env, capture_output=True, text=True, timeout=60,
                         input="".join(json.dumps(m) + "\n" for m in msgs))
    step("mcp: marker captured over stdio", "Knowledge captured" in cap.stdout)
    with open(env["CODEX_HOME"] + "/config.toml", "w") as f:
        f.write('model = "mock-model"\nmodel_provider = "mock"\n[model_providers.mock]\nname = "mock"\n'
                'base_url = "http://127.0.0.1:8787/v1"\nwire_api = "responses"\nenv_key = "OPENAI_API_KEY"\n')
    mock = subprocess.Popen(["python3", "/t/mock_responses.py", "8787", "/tmp/requests.log"])
    time.sleep(1)
    try:
        r = run([codex, *run_argv[:-1], "-o", "/tmp/answer.txt", run_argv[-1]], cwd="/work")
    finally:
        mock.kill()
    calls = []
    for p in glob.glob(env["CODEX_HOME"] + "/sessions/**/rollout-*.jsonl", recursive=True):
        for line in open(p):
            item = (json.loads(line).get("payload") or {}).get("item") or {}
            if item.get("type") == "McpToolCall" and item.get("tool") == "forge_get_knowledge":
                calls.append(item.get("status") + (" (" + item["error"]["message"] + ")" if item.get("error") else ""))
    step("mcp: forge_get_knowledge call completed", calls == ["completed"], ", ".join(calls) or "no call")
    answer = open("/tmp/answer.txt").read() if os.path.exists("/tmp/answer.txt") else ""
    step("mcp: answer carries the marker from forge", MARKER in answer)

sys.exit(1 if failed else 0)
