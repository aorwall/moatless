# Storybook

Read [button stories](../../../../apps/frontend/src/lib/components/ui/__stories__/button.stories.tsx)
or [sidebar-dialog stories](../../../../apps/frontend/src/lib/components/ui/__stories__/sidebar-dialog.stories.tsx)
for the current imports, metadata, decorators, and controls.

Use typed `Meta`/`StoryObj` with the installed `@storybook/react-vite` integration.
Metadata requires a default export; application named-export guidance permits
this framework convention. Stories belong beside the component in `__stories__`.
Cover meaningful states and accessible interactions, with the same providers and
semantic tokens as the app. Do not copy package versions or test imports from an
upstream tutorial without checking this workspace's dependencies.
