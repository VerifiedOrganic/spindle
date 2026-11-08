#!/usr/bin/env python3
"""Check live stdio discovery against registry snapshots in isolated workspaces."""
import argparse
import json
import os
from pathlib import Path
import queue
import subprocess
import tempfile
import threading
import time

ROOT = Path(__file__).resolve().parents[1]
SNAPSHOTS = ROOT / 'crates/spindle-mcp/src/tools/snapshots'


def compact(value):
    return json.dumps(value, ensure_ascii=False, separators=(',', ':'), sort_keys=True).encode()


def measure(tools):
    inputs = [{k: tool[k] for k in ('name', 'description', 'inputSchema')} for tool in tools]
    return len(tools), len(compact(inputs)), len(compact(tools))


def check_budget(tools, budget):
    actual = measure(tools)
    caps = (budget['tools'], budget['input_bytes'], budget['full_bytes'])
    if any(size > cap for size, cap in zip(actual, caps)):
        sizes = sorted(((len(compact(tool)), tool['name']) for tool in tools), reverse=True)
        raise AssertionError(f'catalog sizes {actual} exceed caps {caps}; per-tool full bytes: {sizes}')


def probe(binary, profile, request_timeout=30):
    with tempfile.TemporaryDirectory(prefix='spindle-profile-', dir='/tmp') as directory:
        config = Path(directory) / 'config.toml'
        config.write_text('')
        env = {k: v for k, v in os.environ.items() if not k.startswith('SPINDLE_')}
        env.update(SPINDLE_DATA_DIR=directory, SPINDLE_CONFIG=str(config))
        if profile != 'default':
            env['SPINDLE_TOOL_PROFILE'] = profile
        with tempfile.TemporaryFile() as errors:
            child = subprocess.Popen([str(binary)], cwd=directory, env=env,
                                     stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=errors)
            responses = queue.Queue()

            def read():
                for line in child.stdout:
                    responses.put(line)
                responses.put(None)

            reader = threading.Thread(target=read, daemon=True)
            reader.start()

            def send(message):
                child.stdin.write(compact(message) + b'\n')
                child.stdin.flush()

            def request(identifier, method, params):
                send(dict(jsonrpc='2.0', id=identifier, method=method, params=params))
                deadline = time.monotonic() + request_timeout
                while True:
                    remaining = deadline - time.monotonic()
                    if remaining <= 0:
                        raise TimeoutError(f'{profile}: timed out waiting for {method}')
                    try:
                        line = responses.get(timeout=remaining)
                    except queue.Empty as exc:
                        raise TimeoutError(f'{profile}: timed out waiting for {method}') from exc
                    if line is None:
                        raise RuntimeError(f'{profile}: server exited before {method}')
                    response = json.loads(line)
                    if response.get('id') == identifier:
                        if 'error' in response:
                            raise RuntimeError(response['error'])
                        return response['result']
            try:
                request(1, 'initialize', dict(protocolVersion='2024-11-05', capabilities={},
                                             clientInfo=dict(name='profile-guard', version='1')))
                send(dict(jsonrpc='2.0', method='notifications/initialized'))
                result = request(2, 'tools/list', {})
                child.stdin.close()
                child.wait(timeout=10)
                if child.returncode:
                    raise RuntimeError(f'{profile}: server exit {child.returncode}')
                return result
            finally:
                if child.poll() is None:
                    child.kill()
                    child.wait(timeout=10)
                child.stdin.close()
                child.stdout.close()
                reader.join(timeout=5)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, default=ROOT / 'target/debug/spindle-mcp')
    args = parser.parse_args()
    budgets = json.loads((ROOT / 'tools/profile-budgets.json').read_text())
    assert budgets['version'] == 1
    for profile in ('default', 'minimal', 'write', 'import', 'authoring', 'unknown-profile'):
        expected = json.loads((SNAPSHOTS / f'{"default" if profile == "unknown-profile" else profile}.json').read_text())
        actual = probe(args.binary.resolve(), profile)
        budget_profile = 'default' if profile == 'unknown-profile' else profile
        check_budget(actual['tools'], budgets['compatibility'][budget_profile])
        count, inputs, full = measure(actual['tools'])
        print(f'{profile}: {count} tools; input {inputs} bytes ({(inputs + 3)//4} tokens); '
              f'full {full} bytes ({(full + 3)//4} tokens)')
        if compact(actual) != compact(expected):
            old = {t['name']: t for t in expected['tools']}
            for tool in actual['tools']:
                previous = old.pop(tool['name'], None)
                if compact(tool) != compact(previous):
                    print(f'  {tool["name"]}: {len(compact(previous)) if previous else 0} -> {len(compact(tool))} bytes')
            for name in old:
                print(f'  removed: {name}')
            raise AssertionError(f'{profile}: tools/list differs from registry snapshot')


if __name__ == '__main__':
    main()
