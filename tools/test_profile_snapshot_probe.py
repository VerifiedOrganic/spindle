"""Probe tests use fake stdio servers; no user database or model routes."""
import os
import subprocess
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

from profile_snapshot_probe import check_budget, check_unknown_profile_rejected, compact, measure, probe


class ProbeTests(unittest.TestCase):
    def server(self, directory, body):
        path = Path(directory) / 'fake-server'
        path.write_text('#!/usr/bin/env python3\n' + body)
        path.chmod(0o700)
        return path

    def test_measure_uses_utf8_and_excludes_output_only_from_input(self):
        tool = dict(name='é', description='meaning', inputSchema={}, outputSchema={'description': 'x' * 5000})
        count, inputs, full = measure([tool])
        self.assertEqual(count, 1)
        self.assertEqual(inputs, len(compact([{k: tool[k] for k in ('name', 'description', 'inputSchema')}])))
        self.assertGreater(full - inputs, 5000)

    def test_budget_rejects_input_and_output_growth_independently(self):
        tool = dict(name='small', description='A useful operation.', inputSchema={})
        _, inputs, full = measure([tool])
        budget = dict(tools=1, input_bytes=inputs, full_bytes=full)
        check_budget([tool], budget)
        for field in ('description', 'outputSchema'):
            grown = dict(tool)
            grown[field] = 'x' * 5000
            with self.assertRaisesRegex(AssertionError, 'per-tool full bytes'):
                check_budget([grown], budget)
        with self.assertRaises(AssertionError):
            check_budget([tool, tool], budget)

    def test_isolated_environment_and_protocol(self):
        with tempfile.TemporaryDirectory() as directory:
            binary = self.server(directory, '''import json, os, pathlib, sys
assert 'SPINDLE_HTTP_ADDR' not in os.environ
assert 'SPINDLE_AGENT_CONFIG' not in os.environ
assert os.environ['SPINDLE_TOOL_PROFILE'] == 'minimal'
assert pathlib.Path(os.environ['SPINDLE_CONFIG']).read_text() == ''
assert pathlib.Path(os.environ['SPINDLE_DATA_DIR']).resolve() == pathlib.Path.cwd().resolve()
for line in sys.stdin:
    msg = json.loads(line)
    if msg['method'] == 'initialize':
        print(json.dumps({'jsonrpc': '2.0', 'id': msg['id'], 'result': {}}), flush=True)
    elif msg['method'] == 'tools/list':
        print(json.dumps({'jsonrpc': '2.0', 'id': msg['id'], 'result': {'tools': []}}), flush=True)
''')
            with patch.dict(os.environ, {'SPINDLE_HTTP_ADDR': 'bad', 'SPINDLE_AGENT_CONFIG': 'bad'}):
                self.assertEqual(probe(binary, 'minimal'), {'tools': []})

    def test_stalled_child_is_killed_and_workspace_removed(self):
        with tempfile.TemporaryDirectory() as directory:
            binary = self.server(directory, 'import time\ntime.sleep(60)\n')
            spawn = subprocess.Popen
            children = []
            workspaces = []

            def observe_spawn(*args, **kwargs):
                child = spawn(*args, **kwargs)
                children.append(child)
                workspaces.append(Path(kwargs['cwd']))
                return child

            with patch('profile_snapshot_probe.subprocess.Popen', side_effect=observe_spawn):
                with self.assertRaises(TimeoutError):
                    probe(binary, 'default', request_timeout=1)
            self.assertEqual(len(children), 1)
            self.assertIsNotNone(children[0].poll())
            self.assertFalse(workspaces[0].exists())

    def test_unknown_profile_guard_requires_rejection_before_data_access(self):
        with tempfile.TemporaryDirectory() as directory:
            binary = self.server(directory, "import sys\nprint('unknown tool profile', file=sys.stderr)\nraise SystemExit(1)\n")
            check_unknown_profile_rejected(binary)
            binary = self.server(directory, 'raise SystemExit(0)\n')
            with self.assertRaises(AssertionError):
                check_unknown_profile_rejected(binary)

    def test_early_exit_fails_instead_of_hanging(self):
        with tempfile.TemporaryDirectory() as directory:
            binary = self.server(directory, 'raise SystemExit(1)\n')
            with self.assertRaises((RuntimeError, BrokenPipeError)):
                probe(binary, 'default')


if __name__ == '__main__':
    unittest.main()
