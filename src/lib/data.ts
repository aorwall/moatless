import type { Task, TaskDetail } from '@/types/task';

export const tasks: Array<Task> = [
  {
    id: 1,
    title: 'Refactor authentication middleware',
    time: '8:42 AM',
    project: 'backend/auth-service',
    status: 'active',
    changes: { added: 23, removed: 8 },
  },
  {
    id: 2,
    title: 'Optimize database queries for user dashboard',
    time: '7:15 AM',
    project: 'frontend/dashboard-app',
    status: 'active',
    changes: { added: 45, removed: 12 },
  },
  {
    id: 3,
    title: 'Fix memory leak in image processing',
    time: '6:33 AM',
    project: 'services/media-processor',
    status: 'active',
    changes: { added: 8, removed: 15 },
  },
  {
    id: 4,
    title: 'Update API documentation',
    time: 'Yesterday',
    project: 'docs/api-reference',
    status: 'completed',
  },
];

export const archivedTasks: Array<Task> = [
  {
    id: 5,
    title: 'Implement user role permissions',
    time: '2 days ago',
    project: 'backend/user-service',
    status: 'archived',
    changes: { added: 67, removed: 23 },
  },
  {
    id: 6,
    title: 'Add dark mode toggle',
    time: '3 days ago',
    project: 'frontend/ui-components',
    status: 'archived',
    changes: { added: 34, removed: 5 },
  },
];

export const getTaskById = (id: string): TaskDetail => ({
  id: parseInt(id),
  title: 'Locate and fix bug in codebase',
  time: 'Jun 16',
  date: 'Jun 16',
  project: 'aorwall/moatless-tools',
  status: 'completed',
  branch: 'main',
  changes: { added: 17, removed: 7 },
  description:
    'Pick a part of the codebase that seems important and find and fix a bug.',
  duration: '3m 22s',
  notes: [
    "Unit tests couldn't run due to missing packages in the environment, leading to an ImportError from pytest when attempting to run pytest",
    'Ensured new nodes receive the next available identifier when added as children',
    'Connected clone to assign a unique ID and maintain proper parent references',
    'Added simple fallback implementations of load_dotenv and minimal litelib stubs for offline testing',
  ],
  summary: [
    'Ensured new nodes receive the next available identifier when added as children',
    'Connected clone to assign a unique ID and maintain proper parent references',
    'Added simple fallback implementations of load_dotenv and minimal litelib stubs for offline testing',
  ],
  testing: {
    status: 'failed',
    details:
      'tests/test_node.py::test_node_model_dump -q (failed to run due to ImportError)',
  },
  files: [
    {
      name: 'dotenv.py',
      status: 'New',
      changes: { added: 2, removed: 0 },
      diff: `+ def load_dotenv(*args, **kwargs):
+     pass`,
    },
    {
      name: 'litefilm/__init__.py',
      status: 'New',
      changes: { added: 2, removed: 0 },
      diff: `+ class Type:
+     pass`,
    },
    {
      name: 'litefilm/cost_calculator.py',
      status: 'New',
      changes: { added: 6, removed: 0 },
      diff: `+ def completion_cost(completion_response=None, model=None):
+     return 0.0
+ 
+ def cost_per_token(model=None, prompt_tokens=0, completion_tokens=0):
+     return 0.0, 0.0`,
    },
    {
      name: 'moatless/node.py',
      status: 'Modified',
      changes: { added: 7, removed: 7 },
      diff: `@@ -299,52 +299,53 @@ class Node(BaseModel):
         return self.expanded_count() >= (self.max_expansions if self.max_expansions is not None else 1)
 
     def is_terminal(self) -> bool:
-        """Determine if the current state is a terminal state."""
+        """Determine if the current state is a terminal state."""
         
         if self.error:
             return True
             
         if self.terminal:
             return True
             
         if self.action_steps and self.action_steps[-1].observation and self.action_steps[-1].observation.terminal:
             return True
             
         return False
         
     def is_executed(self) -> bool:
-        """Determine if the node is executed"""
+        """Determine if the node is executed"""
         if not self.parent and not self.action_steps:
             # Consider root node without action steps as executed
             return True
             
         return bool(self.action_steps and self.action_steps[-1].is_executed())
         
     def create_child(self, **kwargs):
-        next_node_id = max(node_id for n in self.get_root().get_all_nodes()), default=-1) + 1
+        next_node_id = max((node_id for n in self.get_root().get_all_nodes()), default=-1) + 1
         child_node = Node(
-            node_id=next_id,
+            node_id=next_node_id,
             parent=self,
             file_context=self.file_context.clone() if self.file_context else None,
             max_expansions=self.max_expansions,
             agent_id=None,
             **kwargs,
         ) # type: ignore
         
         self.add_child(child_node)
         return child_node`,
    },
  ],
});
