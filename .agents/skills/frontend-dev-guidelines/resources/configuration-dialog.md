# Configuration dialogs

Use [SidebarDialog](../../../../apps/frontend/src/lib/components/ui/sidebar-dialog.tsx)
for settings with multiple sections. Its
[stories](../../../../apps/frontend/src/lib/components/ui/__stories__/sidebar-dialog.stories.tsx)
show the supported compound components and responsive behavior.

Read the component's props before composing Sidebar, search, navigation, or
content. Keep selected-section state with the feature and preserve form values
when navigating. Use accessible labels, validation feedback, and explicit save
state. Do not add a second mobile layout or redefine the component API in a
feature. For a small single-purpose form, use the existing dialog primitive.
