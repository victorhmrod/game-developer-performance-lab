import subprocess
import sys

for executable in sys.argv[1:]:
    for response in [
        b'{"version":1,"id":1,"ok":true,"capabilities":[],"metric_definitions":{} trailing}\n',
        b'{"version":1,"id":1,"id":1,"ok":true,"capabilities":[],"metric_definitions":{}}\n',
        b'{"version":1,"id":1',
        b'x' * 8193 + b'\n',
    ]:
        process = subprocess.Popen([executable], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        assert process.stdout.readline().endswith(b'\n')
        process.stdin.write(response)
        process.stdin.close()
        assert process.wait(timeout=7) != 0
    process = subprocess.Popen([executable], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    for index in range(1, 4):
        assert process.stdout.readline().endswith(b'\n')
        if index == 1:
            response = b'{"version":1,"id":1,"ok":true,"capabilities":[],"metric_definitions":{}}\n'
        elif index == 2:
            response = b'{"version":1,"id":2,"ok":true,"scenarios":[]}\n'
        else:
            response = b'{"version":1,"id":3,"ok":true}\n'
        for chunk in (response[:5], response[5:]):
            process.stdin.write(chunk)
            process.stdin.flush()
    assert process.wait(timeout=7) == 0, process.stderr.read()
    process = subprocess.Popen([executable], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    assert process.stdout.readline().endswith(b'\n')
    assert process.wait(timeout=7) != 0
    process.stdin.close()
    print(f'{executable}: malformed, duplicate, truncated, oversized, fragmented and timeout checks passed')
