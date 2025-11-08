import type { Meta, StoryObj } from '@storybook/react'
import { fn } from '@storybook/test'
import { Bot, Database, FolderGit2, Github, Plus, Settings } from 'lucide-react'
import { useState } from 'react'
import { Button } from '../button'
import { SidebarDialog } from '../sidebar-dialog'

const meta = {
  title: 'UI/SidebarDialog',
  component: SidebarDialog,
  parameters: {
    layout: 'fullscreen',
    docs: {
      description: {
        component:
          'A responsive dialog with sidebar navigation. On desktop (≥640px), shows a fixed sidebar. On mobile (<640px), the sidebar becomes a Sheet overlay accessed via hamburger menu, and the dialog goes full-screen.',
      },
    },
  },
  decorators: [
    (Story) => (
      <div className="w-full h-screen bg-background">
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof SidebarDialog>

export default meta

type Story = StoryObj<typeof meta>

// Default: Basic sidebar with navigation
export const Default: Story = {
  render: () => {
    const [open, setOpen] = useState(true)
    const [active, setActive] = useState('general')

    return (
      <SidebarDialog open={open} onOpenChange={setOpen}>
        <SidebarDialog.Sidebar title="Settings" description="Configure your preferences">
          <SidebarDialog.Nav>
            <SidebarDialog.NavItem
              active={active === 'general'}
              onClick={() => setActive('general')}
              icon={<Settings className="h-4 w-4" />}
            >
              General
            </SidebarDialog.NavItem>
            <SidebarDialog.NavItem
              active={active === 'claude'}
              onClick={() => setActive('claude')}
              icon={<Bot className="h-4 w-4" />}
            >
              Claude
            </SidebarDialog.NavItem>
            <SidebarDialog.NavItem
              active={active === 'github'}
              onClick={() => setActive('github')}
              icon={<Github className="h-4 w-4" />}
            >
              GitHub
            </SidebarDialog.NavItem>
          </SidebarDialog.Nav>
        </SidebarDialog.Sidebar>

        <SidebarDialog.Content title="General Settings">
          <div className="space-y-4">
            <p className="text-sm text-muted-foreground">
              Configure your general application settings here.
            </p>
            <div className="rounded-md border p-4">
              <h3 className="font-medium mb-2">Example Setting</h3>
              <p className="text-sm text-muted-foreground">
                This is where your settings form would go.
              </p>
            </div>
          </div>
        </SidebarDialog.Content>
      </SidebarDialog>
    )
  },
}

// WithSearch: Sidebar with search functionality
export const WithSearch: Story = {
  render: () => {
    const [open, setOpen] = useState(true)
    const [active, setActive] = useState('repo-1')
    const [search, setSearch] = useState('')

    const repositories = [
      { id: 'repo-1', name: 'moatless-swarm-ui' },
      { id: 'repo-2', name: 'claude-code' },
      { id: 'repo-3', name: 'frontend-toolkit' },
      { id: 'repo-4', name: 'backend-api' },
    ]

    const filtered = repositories.filter((repo) =>
      repo.name.toLowerCase().includes(search.toLowerCase()),
    )

    return (
      <SidebarDialog open={open} onOpenChange={setOpen}>
        <SidebarDialog.Sidebar title="Repositories" description="Manage your repositories">
          <SidebarDialog.Search
            value={search}
            onChange={setSearch}
            placeholder="Search repositories..."
          />

          <SidebarDialog.Nav emptyMessage="No repositories found">
            {filtered.map((repo) => (
              <SidebarDialog.NavItem
                key={repo.id}
                active={active === repo.id}
                onClick={() => setActive(repo.id)}
                icon={<FolderGit2 className="h-4 w-4" />}
              >
                {repo.name}
              </SidebarDialog.NavItem>
            ))}
          </SidebarDialog.Nav>
        </SidebarDialog.Sidebar>

        <SidebarDialog.Content title={repositories.find((r) => r.id === active)?.name}>
          <div className="space-y-4">
            <p className="text-sm text-muted-foreground">
              Repository configuration content goes here.
            </p>
          </div>
        </SidebarDialog.Content>
      </SidebarDialog>
    )
  },
}

// WithFooter: Sidebar with footer action
export const WithFooter: Story = {
  render: () => {
    const [open, setOpen] = useState(true)
    const [active, setActive] = useState('repo-1')

    const repositories = [
      { id: 'repo-1', name: 'moatless-swarm-ui' },
      { id: 'repo-2', name: 'claude-code' },
    ]

    return (
      <SidebarDialog open={open} onOpenChange={setOpen}>
        <SidebarDialog.Sidebar title="Repositories">
          <SidebarDialog.Nav>
            {repositories.map((repo) => (
              <SidebarDialog.NavItem
                key={repo.id}
                active={active === repo.id}
                onClick={() => setActive(repo.id)}
                icon={<FolderGit2 className="h-4 w-4" />}
              >
                {repo.name}
              </SidebarDialog.NavItem>
            ))}
          </SidebarDialog.Nav>

          <SidebarDialog.Footer>
            <Button className="w-full" size="sm" onClick={fn()}>
              <Plus className="h-4 w-4 mr-2" />
              Add Repository
            </Button>
          </SidebarDialog.Footer>
        </SidebarDialog.Sidebar>

        <SidebarDialog.Content title={repositories.find((r) => r.id === active)?.name}>
          <div className="space-y-4">
            <p className="text-sm text-muted-foreground">Repository settings here.</p>
          </div>
        </SidebarDialog.Content>
      </SidebarDialog>
    )
  },
}

// DynamicContent: Different content for each section
export const DynamicContent: Story = {
  render: () => {
    const [open, setOpen] = useState(true)
    const [active, setActive] = useState('general')

    const sections = [
      { id: 'general', label: 'General', icon: Settings },
      { id: 'database', label: 'Database', icon: Database },
      { id: 'integrations', label: 'Integrations', icon: Github },
    ]

    const renderContent = () => {
      switch (active) {
        case 'general':
          return (
            <div className="space-y-4">
              <h3 className="font-semibold">General Settings</h3>
              <p className="text-sm text-muted-foreground">
                Configure your general application preferences.
              </p>
            </div>
          )
        case 'database':
          return (
            <div className="space-y-4">
              <h3 className="font-semibold">Database Configuration</h3>
              <p className="text-sm text-muted-foreground">Manage your database connections.</p>
            </div>
          )
        case 'integrations':
          return (
            <div className="space-y-4">
              <h3 className="font-semibold">Third-Party Integrations</h3>
              <p className="text-sm text-muted-foreground">Connect external services.</p>
            </div>
          )
        default:
          return null
      }
    }

    return (
      <SidebarDialog open={open} onOpenChange={setOpen}>
        <SidebarDialog.Sidebar title="Settings" description="Configure your application">
          <SidebarDialog.Nav>
            {sections.map(({ id, label, icon: Icon }) => (
              <SidebarDialog.NavItem
                key={id}
                active={active === id}
                onClick={() => setActive(id)}
                icon={<Icon className="h-4 w-4" />}
              >
                {label}
              </SidebarDialog.NavItem>
            ))}
          </SidebarDialog.Nav>
        </SidebarDialog.Sidebar>

        <SidebarDialog.Content title={sections.find((s) => s.id === active)?.label}>
          {renderContent()}
        </SidebarDialog.Content>
      </SidebarDialog>
    )
  },
}

// LongList: Test scrolling with many items
export const LongList: Story = {
  render: () => {
    const [open, setOpen] = useState(true)
    const [active, setActive] = useState('item-1')

    const items = Array.from({ length: 20 }, (_, i) => ({
      id: `item-${i + 1}`,
      name: `Repository ${i + 1}`,
    }))

    return (
      <SidebarDialog open={open} onOpenChange={setOpen}>
        <SidebarDialog.Sidebar title="Repositories" description="Long list of repositories">
          <SidebarDialog.Nav>
            {items.map((item) => (
              <SidebarDialog.NavItem
                key={item.id}
                active={active === item.id}
                onClick={() => setActive(item.id)}
                icon={<FolderGit2 className="h-4 w-4" />}
              >
                {item.name}
              </SidebarDialog.NavItem>
            ))}
          </SidebarDialog.Nav>
        </SidebarDialog.Sidebar>

        <SidebarDialog.Content title={items.find((i) => i.id === active)?.name}>
          <div className="space-y-4">
            <p className="text-sm text-muted-foreground">Content for {active}</p>
          </div>
        </SidebarDialog.Content>
      </SidebarDialog>
    )
  },
}

// Empty: No navigation items
export const Empty: Story = {
  render: () => {
    const [open, setOpen] = useState(true)

    return (
      <SidebarDialog open={open} onOpenChange={setOpen}>
        <SidebarDialog.Sidebar title="Repositories" description="No repositories configured">
          <SidebarDialog.Nav emptyMessage="No repositories found. Click 'Add Repository' to get started." />

          <SidebarDialog.Footer>
            <Button className="w-full" size="sm" onClick={fn()}>
              <Plus className="h-4 w-4 mr-2" />
              Add Repository
            </Button>
          </SidebarDialog.Footer>
        </SidebarDialog.Sidebar>

        <SidebarDialog.Content>
          <div className="flex flex-col items-center justify-center h-64 text-center">
            <FolderGit2 className="h-12 w-12 text-muted-foreground mb-4" />
            <h3 className="font-semibold mb-2">No Repository Selected</h3>
            <p className="text-sm text-muted-foreground">
              Select a repository from the sidebar or create a new one.
            </p>
          </div>
        </SidebarDialog.Content>
      </SidebarDialog>
    )
  },
}

// SearchAndFooter: Combined search and footer
export const SearchAndFooter: Story = {
  render: () => {
    const [open, setOpen] = useState(true)
    const [active, setActive] = useState('repo-1')
    const [search, setSearch] = useState('')

    const repositories = [
      { id: 'repo-1', name: 'moatless-swarm-ui' },
      { id: 'repo-2', name: 'claude-code' },
      { id: 'repo-3', name: 'frontend-toolkit' },
    ]

    const filtered = repositories.filter((repo) =>
      repo.name.toLowerCase().includes(search.toLowerCase()),
    )

    return (
      <SidebarDialog open={open} onOpenChange={setOpen} className="sm:max-w-[1400px]">
        <SidebarDialog.Sidebar title="Repositories" description="Manage your repositories">
          <SidebarDialog.Search
            value={search}
            onChange={setSearch}
            placeholder="Search repositories..."
          />

          <SidebarDialog.Nav>
            {filtered.map((repo) => (
              <SidebarDialog.NavItem
                key={repo.id}
                active={active === repo.id}
                onClick={() => setActive(repo.id)}
                icon={<FolderGit2 className="h-4 w-4" />}
              >
                {repo.name}
              </SidebarDialog.NavItem>
            ))}
          </SidebarDialog.Nav>

          <SidebarDialog.Footer>
            <Button className="w-full" size="sm" onClick={fn()}>
              <Plus className="h-4 w-4 mr-2" />
              Add Repository
            </Button>
          </SidebarDialog.Footer>
        </SidebarDialog.Sidebar>

        <SidebarDialog.Content title={repositories.find((r) => r.id === active)?.name}>
          <div className="space-y-4">
            <p className="text-sm text-muted-foreground">
              Full-featured repository configuration with search and add functionality.
            </p>
          </div>
        </SidebarDialog.Content>
      </SidebarDialog>
    )
  },
}

// MobileViewport: Demonstrates mobile behavior
export const MobileViewport: Story = {
  parameters: {
    viewport: {
      defaultViewport: 'mobile1',
    },
    docs: {
      description: {
        story:
          'On mobile viewports, the dialog goes full-screen and the sidebar is hidden. Click the hamburger menu icon (☰) to open the sidebar as an overlay Sheet.',
      },
    },
  },
  render: () => {
    const [open, setOpen] = useState(true)
    const [active, setActive] = useState('general')

    return (
      <SidebarDialog open={open} onOpenChange={setOpen}>
        <SidebarDialog.Sidebar title="Settings" description="Mobile view demonstration">
          <SidebarDialog.Nav>
            <SidebarDialog.NavItem
              active={active === 'general'}
              onClick={() => setActive('general')}
              icon={<Settings className="h-4 w-4" />}
            >
              General
            </SidebarDialog.NavItem>
            <SidebarDialog.NavItem
              active={active === 'github'}
              onClick={() => setActive('github')}
              icon={<Github className="h-4 w-4" />}
            >
              GitHub
            </SidebarDialog.NavItem>
            <SidebarDialog.NavItem
              active={active === 'database'}
              onClick={() => setActive('database')}
              icon={<Database className="h-4 w-4" />}
            >
              Database
            </SidebarDialog.NavItem>
          </SidebarDialog.Nav>
        </SidebarDialog.Sidebar>

        <SidebarDialog.Content title="Settings">
          <div className="space-y-4">
            <h3 className="font-semibold">Mobile Experience</h3>
            <p className="text-sm text-muted-foreground">• Full-screen dialog on mobile devices</p>
            <p className="text-sm text-muted-foreground">
              • Hamburger menu (☰) in top-left opens sidebar
            </p>
            <p className="text-sm text-muted-foreground">• Sidebar slides in as overlay Sheet</p>
            <p className="text-sm text-muted-foreground">
              • Clicking a nav item closes the sidebar
            </p>
          </div>
        </SidebarDialog.Content>
      </SidebarDialog>
    )
  },
}
