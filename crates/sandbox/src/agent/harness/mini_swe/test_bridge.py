"""Native history repair and descendant cleanup contracts for the bridge."""
import importlib.util
import os
import tempfile
import time
import unittest
from pathlib import Path

os.environ['MSWEA_GLOBAL_CONFIG_DIR'] = tempfile.mkdtemp(prefix='mini-config-')
spec = importlib.util.spec_from_file_location('mini_bridge', Path(__file__).with_name('bridge.py'))
bridge = importlib.util.module_from_spec(spec)
spec.loader.exec_module(bridge)


class Model:
    def format_observation_messages(self, message, outputs, variables):
        return [{'type': 'function_call_output', 'call_id': a['tool_call_id'], 'output': o['output']}
                for a, o in zip(message['extra']['actions'], outputs)]

    def get_template_vars(self):
        return {}


class BridgeTests(unittest.TestCase):
    def test_repair_preserves_answered_calls_and_closes_only_missing_results(self):
        with tempfile.TemporaryDirectory() as directory:
            agent = bridge.Agent(Model(), bridge.Environment(cwd=directory),
                                 journal=Path(directory)/'journal', model_name='openai/test',
                                 system_template='', instance_template='')
            agent.messages = [{'extra': {'actions': [
                {'tool_call_id': 'answered', 'command': 'true'},
                {'tool_call_id': 'missing', 'command': 'sleep 60'}]}},
                {'type': 'function_call_output', 'call_id': 'answered', 'output': 'original'}]
            agent.repair()
            self.assertEqual(len(agent.messages), 3)
            self.assertEqual(agent.messages[-1]['call_id'], 'missing')
            self.assertEqual(agent.messages[1]['output'], 'original')
            agent.repair()
            self.assertEqual(len(agent.messages), 3)

    def test_provider_conversion_preserves_calls_results_and_source_history(self):
        messages = [{'object': 'response', 'output': [
            {'type': 'function_call', 'call_id': 'c1', 'name': 'bash', 'arguments': '{"command":"pwd"}'}],
            'extra': {'actions': [{'tool_call_id': 'c1', 'command': 'pwd'}]}},
            {'type': 'function_call_output', 'call_id': 'c1', 'output': '/workspace', 'extra': {}}]
        chat = bridge.model_messages(messages, 'anthropic')
        self.assertEqual(chat[0]['tool_calls'][0]['id'], 'c1')
        self.assertEqual(chat[1]['tool_call_id'], 'c1')
        responses = bridge.model_messages(chat, 'openai')
        self.assertEqual(responses, messages)
        self.assertEqual(messages[0]['object'], 'response')

    @unittest.skipUnless(os.name == 'posix', 'process group cleanup is POSIX')
    def test_timeout_kills_a_tool_descendant_before_its_side_effect(self):
        with tempfile.TemporaryDirectory() as directory:
            env = bridge.Environment(cwd=directory, timeout=1)
            result = env.execute({'command': '(sleep 2; echo BAD > escaped) & wait'})
            self.assertEqual(result['returncode'], -1)
            time.sleep(2)
            self.assertFalse((Path(directory)/'escaped').exists())

    def test_submission_closes_all_tool_calls_before_the_exit(self):
        with tempfile.TemporaryDirectory() as directory:
            agent = bridge.Agent(Model(), bridge.Environment(cwd=directory),
                                 journal=Path(directory)/'journal', model_name='openai/test',
                                 system_template='', instance_template='')
            message = {'extra': {'actions': [
                {'tool_call_id': 'finish', 'command': "printf 'COMPLETE_TASK_AND_SUBMIT_FINAL_OUTPUT\\nDONE'"},
                {'tool_call_id': 'late', 'command': 'touch escaped'}]}}
            with self.assertRaises(bridge.Submitted):
                agent.execute_actions(message)
            self.assertEqual([m['call_id'] for m in agent.messages], ['finish', 'late'])
            self.assertFalse((Path(directory)/'escaped').exists())


if __name__ == '__main__':
    unittest.main()
