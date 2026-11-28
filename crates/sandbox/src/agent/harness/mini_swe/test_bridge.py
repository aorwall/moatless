"""Native history repair, output capping, configuration and process cleanup contracts for the bridge."""
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


    def test_a_large_output_is_capped_and_spilled_to_a_file(self):
        with tempfile.TemporaryDirectory() as directory:
            env = bridge.Environment(cwd=directory, output_limit=100, output_dir=directory + '/out')
            result = env.execute({'command': 'seq 1 100000'})
            self.assertLess(len(result['output']), 300)
            self.assertTrue(result['output'].startswith('1\n2\n'))
            self.assertTrue(result['output'].rstrip().endswith('100000'))
            spilled = list(Path(directory, 'out').iterdir())
            self.assertEqual(len(spilled), 1)
            self.assertIn(str(spilled[0]), result['output'])
            self.assertEqual(spilled[0].read_text().splitlines()[-1], '100000')

    def test_a_call_timeout_is_clamped_to_the_maximum(self):
        with tempfile.TemporaryDirectory() as directory:
            env = bridge.Environment(cwd=directory, timeout=30, max_timeout=1)
            result = env.execute({'command': 'sleep 5', 'timeout': 600})
            self.assertEqual(result['returncode'], -1)
            self.assertIn('1 seconds', result['exception_info'])

    def test_timeouts_are_read_from_the_call_arguments(self):
        actions = bridge.with_timeouts(
            [{'command': 'a', 'tool_call_id': 'x'}, {'command': 'b', 'tool_call_id': 'y'}],
            {'x': '{"command": "a", "timeout": 300}', 'y': '{"command": "b", "timeout": "soon"}'})
        self.assertEqual(actions[0]['timeout'], 300)
        self.assertNotIn('timeout', actions[1])

    def test_restored_history_cuts_oversized_tool_results(self):
        messages = [{'type': 'function_call_output', 'call_id': 'c', 'output': 'x' * 5000,
                     'extra': {'raw_output': 'x' * 5000}},
                    {'role': 'user', 'content': 'y' * 5000}]
        bridge.cap_history(messages, 100)
        self.assertLess(len(messages[0]['output']), 300)
        self.assertLess(len(messages[0]['extra']['raw_output']), 300)
        self.assertEqual(len(messages[1]['content']), 5000)

    def test_text_of_a_reply_without_tool_calls_is_recovered(self):
        chat = {'choices': [{'message': {'content': 'All done.'}}]}
        responses = {'output': [{'type': 'reasoning'}, {'type': 'message', 'content': [
            {'type': 'output_text', 'text': 'All done.'}]}]}
        self.assertEqual(bridge.response_text(chat), 'All done.')
        self.assertEqual(bridge.response_text(responses), 'All done.')
        self.assertEqual(bridge.response_text('<repr>'), '')

    def test_the_moatless_config_builds_both_providers_and_renders_the_prompt(self):
        with tempfile.TemporaryDirectory() as directory:
            token = Path(directory) / 'token'
            token.write_text('t')
            for model, kind in [('anthropic/m', bridge.ChatModel), ('openai/m', bridge.ResponsesModel)]:
                agent, _ = bridge.build({
                    'journal': f'{directory}/j.jsonl', 'model': model, 'api_base': 'http://gw',
                    'token_path': str(token), 'cwd': directory, 'output_dir': directory,
                    'config': Path(__file__).with_name('mini.yaml').read_text(),
                    'system_prompt': 'BASE PROMPT', 'skills': 'SKILL LIST'})
                self.assertIsInstance(agent.model, kind)
                self.assertEqual(agent.env.config.env['PAGER'], 'cat')
                self.assertEqual(agent.model.config.model_kwargs['api_base'], 'http://gw')
                prompt = agent._render_template(agent.config.system_template)
                self.assertTrue(prompt.startswith('BASE PROMPT'))
                self.assertIn('SKILL LIST', prompt)
                self.assertIn('COMPLETE_TASK_AND_SUBMIT_FINAL_OUTPUT', prompt)
                self.assertIn('up to 600', prompt)


if __name__ == '__main__':
    unittest.main()
