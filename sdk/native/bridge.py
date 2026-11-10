import subprocess
import sys
import queue
import threading

# Local example bridge: client stdout -> adapter stdin; adapter stdout -> client stdin.
if len(sys.argv) < 3:
    raise SystemExit("usage: bridge.py CLIENT ADAPTER [ADAPTER_ARGS...]")
client = subprocess.Popen([sys.argv[1]], stdin=subprocess.PIPE, stdout=subprocess.PIPE)
adapter = subprocess.Popen(sys.argv[2:], stdin=subprocess.PIPE, stdout=subprocess.PIPE)
def bounded(operation):
    result = queue.Queue(maxsize=1)
    def run():
        try:
            result.put((True, operation()))
        except Exception as error:
            result.put((False, error))
    threading.Thread(target=run, daemon=True).start()
    try:
        success, value = result.get(timeout=5)
    except queue.Empty as error:
        raise TimeoutError("IPC operation exceeded five seconds") from error
    if not success:
        raise value
    return value

try:
    for _ in range(3):
        request = bounded(lambda: client.stdout.readline(8193))
        if not request.endswith(b"\n"):
            raise RuntimeError("client disconnected or sent an oversized request")
        bounded(lambda: (adapter.stdin.write(request), adapter.stdin.flush()))
        reply = bounded(lambda: adapter.stdout.readline(8193))
        if not reply.endswith(b"\n"):
            raise RuntimeError("adapter disconnected or sent an oversized response")
        bounded(lambda: (client.stdin.write(reply), client.stdin.flush()))
    if client.wait(timeout=10) != 0 or adapter.wait(timeout=10) != 0:
        raise RuntimeError("peer exited unsuccessfully")
finally:
    for process in (client, adapter):
        if process.poll() is None:
            process.kill()
            process.wait()
