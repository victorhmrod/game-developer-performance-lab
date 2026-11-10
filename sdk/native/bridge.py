import subprocess
import sys

# Local example bridge: client stdout -> adapter stdin; adapter stdout -> client stdin.
if len(sys.argv) < 3:
    raise SystemExit("usage: bridge.py CLIENT ADAPTER [ADAPTER_ARGS...]")
client = subprocess.Popen([sys.argv[1]], stdin=subprocess.PIPE, stdout=subprocess.PIPE)
adapter = subprocess.Popen(sys.argv[2:], stdin=subprocess.PIPE, stdout=subprocess.PIPE)
try:
    for _ in range(3):
        request = client.stdout.readline(8193)
        if not request.endswith(b"\n"):
            raise RuntimeError("client disconnected or sent an oversized request")
        adapter.stdin.write(request)
        adapter.stdin.flush()
        reply = adapter.stdout.readline(8193)
        if not reply.endswith(b"\n"):
            raise RuntimeError("adapter disconnected or sent an oversized response")
        client.stdin.write(reply)
        client.stdin.flush()
    if client.wait(timeout=10) != 0 or adapter.wait(timeout=10) != 0:
        raise RuntimeError("peer exited unsuccessfully")
finally:
    for process in (client, adapter):
        if process.poll() is None:
            process.kill()
            process.wait()
