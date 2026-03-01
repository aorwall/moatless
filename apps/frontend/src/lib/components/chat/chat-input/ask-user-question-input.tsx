import { HelpCircle, Loader2, Send } from 'lucide-react'
import { useCallback, useState } from 'react'
import { Button } from '@/lib/components/ui/button'
import { Checkbox } from '@/lib/components/ui/checkbox'
import { Input } from '@/lib/components/ui/input'
import { Label } from '@/lib/components/ui/label'
import type { Question } from '@/lib/types/user-input'
import { cn } from '@/lib/utils'

interface AskUserQuestionInputProps {
  questions: Question[]
  onSubmit: (answers: Record<string, string>) => void
  isPending?: boolean
  error?: Error | null
}

export function AskUserQuestionInput({
  questions,
  onSubmit,
  isPending = false,
  error,
}: AskUserQuestionInputProps) {
  const [answers, setAnswers] = useState<Record<string, string>>({})
  const [otherText, setOtherText] = useState<Record<string, string>>({})

  const handleSubmit = useCallback(() => {
    const finalAnswers: Record<string, string> = {}
    for (const q of questions) {
      const answer = answers[q.question]
      finalAnswers[q.question] = answer === 'Other' ? otherText[q.question] || answer : answer
    }
    onSubmit(finalAnswers)
  }, [questions, answers, otherText, onSubmit])

  const allAnswered = questions.every((q) => answers[q.question])

  return (
    <div className="z-10 flex-shrink-0 bg-background absolute bottom-0 left-0 right-0">
      <div data-testid="ask-user-question-input" className="p-4">
        <div className="group flex flex-col gap-3 rounded-3xl border border-blue-500 bg-card p-4 relative shadow-sm">
          {/* Header */}
          <div className="flex items-center gap-2 text-sm font-medium text-blue-600 dark:text-blue-400">
            <HelpCircle className="h-4 w-4" />
            <span>Claude needs your input</span>
          </div>

          {/* Questions */}
          <div
            className={cn(
              'space-y-4 max-h-[50vh] overflow-y-auto pr-2',
              'scrollbar-thin scrollbar-track-transparent scrollbar-thumb-muted',
            )}
          >
            {questions.map((question) => (
              <div key={question.question} className="space-y-2">
                {question.header && <h4 className="text-sm font-semibold">{question.header}</h4>}
                <p className="text-sm text-muted-foreground">{question.question}</p>

                {question.multiSelect ? (
                  <div className="space-y-2 pl-1">
                    {question.options.map((opt) => {
                      const optId = `input-${question.question}-${opt.label}`
                      return (
                        <div key={opt.label} className="flex items-start gap-2">
                          <Checkbox
                            id={optId}
                            checked={answers[question.question]?.includes(opt.label) ?? false}
                            disabled={isPending}
                            onCheckedChange={(checked) => {
                              const current =
                                answers[question.question]?.split(', ').filter(Boolean) ?? []
                              const updated = checked
                                ? [...current, opt.label].join(', ')
                                : current.filter((l) => l !== opt.label).join(', ')
                              setAnswers((prev) => ({ ...prev, [question.question]: updated }))
                            }}
                          />
                          <Label htmlFor={optId} className="text-sm cursor-pointer">
                            <span className="font-medium">{opt.label}</span>
                            {opt.description && (
                              <span className="text-muted-foreground ml-1">
                                - {opt.description}
                              </span>
                            )}
                          </Label>
                        </div>
                      )
                    })}
                  </div>
                ) : (
                  <div className="space-y-2 pl-1">
                    {question.options.map((opt) => {
                      const optId = `input-${question.question}-${opt.label}`
                      return (
                        <div key={opt.label} className="flex items-start gap-2">
                          <input
                            type="radio"
                            id={optId}
                            name={`input-${question.question}`}
                            checked={answers[question.question] === opt.label}
                            disabled={isPending}
                            onChange={() =>
                              setAnswers((prev) => ({ ...prev, [question.question]: opt.label }))
                            }
                            className="h-4 w-4 mt-0.5 accent-primary"
                          />
                          <Label htmlFor={optId} className="text-sm cursor-pointer">
                            <span className="font-medium">{opt.label}</span>
                            {opt.description && (
                              <span className="text-muted-foreground ml-1">
                                - {opt.description}
                              </span>
                            )}
                          </Label>
                        </div>
                      )
                    })}
                    {/* Other option */}
                    <div className="flex items-start gap-2">
                      <input
                        type="radio"
                        id={`input-${question.question}-other`}
                        name={`input-${question.question}`}
                        checked={answers[question.question] === 'Other'}
                        disabled={isPending}
                        onChange={() =>
                          setAnswers((prev) => ({ ...prev, [question.question]: 'Other' }))
                        }
                        className="h-4 w-4 mt-0.5 accent-primary"
                      />
                      <div className="flex-1 space-y-1">
                        <Label
                          htmlFor={`input-${question.question}-other`}
                          className="text-sm font-medium"
                        >
                          Other
                        </Label>
                        {answers[question.question] === 'Other' && (
                          <Input
                            placeholder="Type your answer..."
                            value={otherText[question.question] || ''}
                            onChange={(e) =>
                              setOtherText((prev) => ({
                                ...prev,
                                [question.question]: e.target.value,
                              }))
                            }
                            disabled={isPending}
                            className="h-8 text-sm"
                            onKeyDown={(e) => {
                              if (e.key === 'Enter' && !e.shiftKey && allAnswered && !isPending) {
                                e.preventDefault()
                                handleSubmit()
                              }
                            }}
                          />
                        )}
                      </div>
                    </div>
                  </div>
                )}
              </div>
            ))}
          </div>

          {/* Error message */}
          {error && (
            <p className="text-sm text-destructive">{error.message || 'Failed to submit'}</p>
          )}

          {/* Submit button */}
          <div className="flex justify-end">
            <Button
              onClick={handleSubmit}
              disabled={!allAnswered || isPending}
              size="sm"
              className="gap-2"
            >
              {isPending ? (
                <>
                  <Loader2 className="h-4 w-4 animate-spin" />
                  Submitting...
                </>
              ) : (
                <>
                  <Send className="h-4 w-4" />
                  Submit Answer{questions.length > 1 ? 's' : ''}
                </>
              )}
            </Button>
          </div>
        </div>
      </div>
    </div>
  )
}
