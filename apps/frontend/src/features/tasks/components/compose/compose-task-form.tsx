import type React from 'react'
import { FigmaImportDialog } from '@/features/figma/components/figma-import-dialog'
import { CreateRepositoryDialog } from '@/features/repositories'
import { SettingsDialog } from '@/features/settings'
import { ContextItems, FileAttachments } from '@/features/tasks/components/chat/chat-input'
import {
  AttachmentPopoverContent,
  BranchPopoverContent,
  PopoverSelector,
  SearchablePopoverContent,
  SimpleListPopoverContent,
} from '@/features/tasks/components/chat/chat-input/selectors'
import { CommandAutocomplete } from '@/features/tasks/components/chat/command-autocomplete'
import {
  type UseComposeTaskFormArgs,
  useComposeTaskForm,
} from '@/features/tasks/hooks/use-compose-task-form'
import { Button } from '@/lib/components/ui/button'
import { Input } from '@/lib/components/ui/input'
import { Textarea } from '@/lib/components/ui/textarea'

type ComposeTaskFormProps = UseComposeTaskFormArgs

/**
 * Gmail-compose-style task creator: stacked labeled rows (Repository / Agent /
 * Name — like To / Cc / Subject) above a large message body and a bottom
 * toolbar. Meant to fill a docked compose window.
 */
export function ComposeTaskForm(props: ComposeTaskFormProps) {
  const { name, onNameChange, message } = props
  const form = useComposeTaskForm(props)

  const handleBodyKeyDown = (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
    form.handleCommandKeyDown(e)
    if (e.defaultPrevented) {
      return
    }
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault()
      form.handleSubmit()
    }
  }

  const showBranch = !(form.branchSelector.hidden || form.isTemplate)

  return (
    <div className="flex h-full flex-col">
      {/* Header rows — the "To / Cc / Subject" of the task */}
      <div className="flex flex-col text-sm">
        <ComposeRow label="Repository">
          <PopoverSelector
            icon={form.repoSelector.icon}
            value={form.repoSelector.value}
            displayValue={form.repoSelector.displayValue}
            tooltip={form.repoSelector.tooltip}
            loading={form.repoSelector.loading}
            disabled={form.repoSelector.disabled}
          >
            <SearchablePopoverContent
              items={form.repoSelector.items}
              value={form.repoSelector.value}
              onChange={form.repoSelector.onChange}
              placeholder="Search repositories…"
              emptyText="No repositories found"
              onEmptyAction={() => form.setRepoDialogOpen(true)}
              emptyActionText="Create Repository"
            />
          </PopoverSelector>

          {showBranch && (
            <PopoverSelector
              icon={form.branchSelector.icon}
              value={form.branchSelector.value}
              displayValue={form.branchSelector.displayValue}
              tooltip={form.branchSelector.tooltip}
              loading={form.branchSelector.loading}
              disabled={form.branchSelector.disabled}
              error={form.branchSelector.isError}
            >
              <BranchPopoverContent
                items={form.branchSelector.items}
                value={form.branchSelector.value}
                onChange={form.branchSelector.onChange}
                currentBranch={form.branchSelector.currentBranch}
                enableSearch={true}
                error={form.branchSelector.error}
              />
            </PopoverSelector>
          )}
        </ComposeRow>

        <ComposeRow label="Agent">
          <PopoverSelector
            icon={form.agentTypeSelector.icon}
            value={form.agentTypeSelector.value}
            displayValue={form.agentTypeSelector.displayValue}
            tooltip={form.agentTypeSelector.tooltip}
            loading={form.agentTypeSelector.loading}
            disabled={form.agentTypeSelector.disabled}
          >
            <SimpleListPopoverContent
              items={form.agentTypeSelector.items}
              value={form.agentTypeSelector.value}
              onChange={form.agentTypeSelector.onChange}
            />
          </PopoverSelector>

          <PopoverSelector
            icon={form.agentModeSelector.icon}
            value={form.agentModeSelector.value}
            displayValue={form.agentModeSelector.displayValue}
            tooltip={form.agentModeSelector.tooltip}
            loading={form.agentModeSelector.loading}
            disabled={form.agentModeSelector.disabled}
          >
            <SimpleListPopoverContent
              items={form.agentModeSelector.items}
              value={form.agentModeSelector.value}
              onChange={form.agentModeSelector.onChange}
            />
          </PopoverSelector>

          {form.isModelSelectorEnabled && (
            <PopoverSelector
              icon={form.modelSelector.icon}
              value={form.modelSelector.value}
              displayValue={form.modelSelector.displayValue}
              tooltip={form.modelSelector.tooltip}
              loading={form.modelSelector.loading}
              disabled={form.modelSelector.disabled}
            >
              <SimpleListPopoverContent
                items={form.modelSelector.items}
                value={form.modelSelector.value}
                onChange={form.modelSelector.onChange}
              />
            </PopoverSelector>
          )}
        </ComposeRow>

        <ComposeRow label="Name">
          <Input
            value={name}
            onChange={(e) => onNameChange(e.target.value)}
            placeholder="Optional — the agent will name it for you"
            className="h-7 border-0 px-0 shadow-none focus-visible:ring-0"
          />
        </ComposeRow>
      </div>

      {/* Message body — like the email body */}
      <div className="relative min-h-0 flex-1">
        <Textarea
          ref={form.textareaRef}
          value={message}
          onChange={(e) => form.handleMessageChange(e.target.value)}
          onKeyDown={handleBodyKeyDown}
          onPaste={form.taskInputs.handlePaste}
          placeholder={form.placeholder}
          className="h-full resize-none border-0 px-0 py-3 shadow-none focus-visible:ring-0"
        />
        {form.isCommandAutocompleteOpen && (
          <CommandAutocomplete
            commands={form.filteredCommands}
            selectedIndex={form.commandSelectedIndex}
            onSelect={form.selectCommand}
            position="above"
          />
        )}
      </div>

      {/* Attachments + agent-unavailable hint */}
      {(form.taskInputs.contextItems.length > 0 || form.taskInputs.figmaImportPending) && (
        <div className="pb-2">
          <ContextItems
            contexts={form.taskInputs.contextItems}
            onRemove={form.taskInputs.handleRemoveContext}
            isLoading={form.taskInputs.figmaImportPending}
          />
        </div>
      )}

      {form.taskInputs.uploadedFiles.length > 0 && (
        <div className="pb-2">
          <FileAttachments
            files={form.taskInputs.uploadedFiles}
            onRemove={form.taskInputs.handleRemoveFile}
          />
        </div>
      )}

      {form.agentUnavailable && (
        <div className="pb-2 text-xs text-muted-foreground">
          {form.selectedAgent?.missingCredentialHint ??
            `Connect ${form.selectedAgent?.label ?? 'this agent'} to run tasks.`}{' '}
          <button
            type="button"
            className="underline hover:text-foreground"
            onClick={() => form.setSettingsDialogOpen(true)}
          >
            Open Settings
          </button>
        </div>
      )}

      {/* Bottom toolbar — Send on the left, attachments on the right */}
      <div className="flex items-center justify-between border-t pt-2">
        <Button
          type="button"
          onClick={form.handleSubmit}
          disabled={!form.canSubmit || form.isPending}
          className="rounded-full px-5"
        >
          {getSubmitLabel(form.isPending, message)}
        </Button>

        <PopoverSelector
          icon={form.taskInputs.attachmentSelector.icon}
          value={form.taskInputs.attachmentSelector.value}
          tooltip={form.taskInputs.attachmentSelector.tooltip}
          loading={form.taskInputs.attachmentSelector.loading}
          disabled={form.taskInputs.attachmentSelector.disabled}
        >
          <AttachmentPopoverContent
            onFileSelect={(file) => form.taskInputs.uploadFile.mutate(file)}
            onFigmaImport={form.handleFigmaImportClick}
            disabled={form.taskInputs.uploadFile.isPending}
          />
        </PopoverSelector>
      </div>

      <CreateRepositoryDialog
        open={form.repoDialogOpen}
        onOpenChange={form.setRepoDialogOpen}
        onCreated={(repositoryId) => form.repoSelector.onChange(repositoryId)}
      />
      <FigmaImportDialog
        open={form.taskInputs.figmaDialogOpen}
        onOpenChange={form.taskInputs.setFigmaDialogOpen}
        importMutation={form.taskInputs.figmaUrlImport}
      />
      <SettingsDialog
        open={form.settingsDialogOpen}
        onOpenChange={form.setSettingsDialogOpen}
        defaultSection={form.settingsSection}
      />
    </div>
  )
}

/** A labeled Gmail-style row: fixed-width muted label + inline field(s). */
function ComposeRow({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="flex min-h-9 items-center gap-2 border-b py-1">
      <span className="w-20 shrink-0 text-xs text-muted-foreground">{label}</span>
      <div className="flex flex-1 flex-wrap items-center gap-2">{children}</div>
    </div>
  )
}

function getSubmitLabel(isPending: boolean, message: string): string {
  if (isPending) {
    return message.trim() ? 'Starting…' : 'Creating…'
  }
  return message.trim() ? 'Start task' : 'Create task'
}
