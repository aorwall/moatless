import { AppBreadcrumbs } from '@/features/navigation/components/breadcrumbs';
import { ThemeToggle } from '@/lib/components/theme-toggle';

export const Header = () => {
  return (
    <div className="flex flex-1 items-center justify-between">
      <AppBreadcrumbs />
      <ThemeToggle />
    </div>
  );
};
