# File-based routing

Routes live in [src/routes](../../../../apps/frontend/src/routes). Read the nearest
route before adding one; [tasks/index.tsx](../../../../apps/frontend/src/routes/tasks/index.tsx)
shows search validation and feature composition.

- Export `Route` through TanStack Router's file-route API. Keep page behavior in
  feature components and hooks.
- Dynamic task segments use `$task-id`. Match generated route parameter names
  exactly in links and navigation.
- Validate search parameters at the route boundary and preserve unrelated filters
  when changing one value.
- Use the existing `.lazy.tsx` convention when splitting route code.
- `routeTree.gen.ts` is generated. Edit route files and let the plugin update it.

Do not paste a route factory for a path that is absent from the generated tree;
its type is tied to the actual route hierarchy.
