import { Sparkles } from 'lucide-react'
import { Selectors, SendButton, StopButton } from '@/features/tasks/components/chat/chat-input'
import {
  AttachmentPopoverContent,
  PopoverSelector,
  SelectorButton,
  SimpleListPopoverContent,
  type useAgentModeSelector,
  type useModelSelector,
} from '@/features/tasks/components/chat/chat-input/selectors'
import type { useTaskInputs } from '@/features/tasks/hooks/use-task-inputs'

interface ChatInputToolbarProps {
  modelSelector: ReturnType<typeof useModelSelector>
  agentModeSelector: ReturnType<typeof useAgentModeSelector>
  isModelSelectorEnabled: boolean
  hideAgentModeSelector: boolean
  startFresh: boolean
  onToggleStartFresh: () => void
  taskInputs: ReturnType<typeof useTaskInputs>
  onFigmaImportClick: () => void
  isTaskRunning: boolean
  onStop: () => void
  isStopPending: boolean
  canSubmit: boolean
  isSendPending: boolean
}

/**
 * Bottom row of the chat input: model/agent-mode/start-fresh selectors on the
 * left, attachment picker + stop/send buttons on the right.
 */
export function ChatInputToolbar({
  modelSelector,
  agentModeSelector,
  isModelSelectorEnabled,
  hideAgentModeSelector,
  startFresh,
  onToggleStartFresh,
  taskInputs,
  onFigmaImportClick,
  isTaskRunning,
  onStop,
  isStopPending,
  canSubmit,
  isSendPending,
}: ChatInputToolbarProps) {
  return (
    <div className="flex items-center justify-between gap-2">
      <Selectors>
        {isModelSelectorEnabled && (
          <PopoverSelector
            icon={modelSelector.icon}
            value={modelSelector.value}
            displayValue={modelSelector.displayValue}
            tooltip={modelSelector.tooltip}
            loading={modelSelector.loading}
            disabled={modelSelector.disabled}
          >
            <SimpleListPopoverContent
              items={modelSelector.items}
              value={modelSelector.value}
              onChange={modelSelector.onChange}
            />
          </PopoverSelector>
        )}

        {!hideAgentModeSelector && (
          <PopoverSelector
            icon={agentModeSelector.icon}
            value={agentModeSelector.value}
            displayValue={agentModeSelector.displayValue}
            tooltip={agentModeSelector.tooltip}
            loading={agentModeSelector.loading}
            disabled={agentModeSelector.disabled}
          >
            <SimpleListPopoverContent
              items={agentModeSelector.items}
              value={agentModeSelector.value}
              onChange={agentModeSelector.onChange}
            />
          </PopoverSelector>
        )}

        <SelectorButton
          icon={Sparkles}
          label={startFresh ? 'Start fresh: on' : 'Start fresh'}
          onClick={onToggleStartFresh}
          className={startFresh ? 'border-primary text-primary' : undefined}
        />
      </Selectors>

      <div className="flex items-center gap-2">
        <PopoverSelector
          icon={taskInputs.attachmentSelector.icon}
          value={taskInputs.attachmentSelector.value}
          displayValue={taskInputs.attachmentSelector.displayValue}
          tooltip={taskInputs.attachmentSelector.tooltip}
          loading={taskInputs.attachmentSelector.loading}
          disabled={taskInputs.attachmentSelector.disabled}
        >
          <AttachmentPopoverContent
            onFileSelect={(file) => taskInputs.uploadFile.mutate(file)}
            onFigmaImport={onFigmaImportClick}
            disabled={taskInputs.uploadFile.isPending}
          />
        </PopoverSelector>
        {isTaskRunning && <StopButton onClick={onStop} disabled={isStopPending} />}
        <SendButton disabled={!canSubmit} loading={isSendPending} />
      </div>
    </div>
  )
}
