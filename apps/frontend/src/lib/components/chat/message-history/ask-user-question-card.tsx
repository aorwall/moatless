import { CheckCircle, HelpCircle, Loader2 } from 'lucide-react'
import { useCallback, useState } from 'react'
import { useTaskId } from '@/features/tasks'
import { useSendMessage } from '@/features/tasks/hooks/use-tasks'
import { Button } from '@/lib/components/ui/button'
import { Card, CardContent, CardHeader, CardTitle } from '@/lib/components/ui/card'
import { Checkbox } from '@/lib/components/ui/checkbox'
import { Input } from '@/lib/components/ui/input'
import { Label } from '@/lib/components/ui/label'
import type { Question } from '@/lib/types/user-input'
import { cn } from '@/lib/utils'

interface Props {
  questions: Question[]
  isAnswered?: boolean
  existingAnswers?: Record<string, string>
  /** True when this is a pending question requiring user input (message has hasPendingUserInput=true) */
  isPendingQuestion?: boolean
}

export function AskUserQuestionCard({
  questions,
  isAnswered = false,
  existingAnswers,
  isPendingQuestion = false,
}: Props) {
  const taskId = useTaskId()
  const [answers, setAnswers] = useState<Record<string, string>>(existingAnswers ?? {})
  const [otherText, setOtherText] = useState<Record<string, string>>({})
  const [submitted, setSubmitted] = useState(isAnswered)
  const submitMutation = useSendMessage(taskId)

  const handleSubmit = useCallback(async () => {
    const finalAnswers: Record<string, string> = {}
    for (const q of questions) {
      const answer = answers[q.question]
      finalAnswers[q.question] = answer === 'Other' ? otherText[q.question] || answer : answer
    }

    try {
      await submitMutation.mutateAsync({
        message: '',
        toolName: 'AskUserQuestion',
        toolResponse: { answers: finalAnswers },
      })
      setSubmitted(true)
    } catch {
      // Error handled by mutation
    }
  }, [questions, answers, otherText, submitMutation])

  const allAnswered = questions.every((q) => answers[q.question])

  return (
    <Card
      className={cn(
        'my-2',
        submitted && 'border-blue-500/30 bg-blue-500/5 opacity-75',
        !submitted && isPendingQuestion && 'border-blue-500 bg-blue-500/10 shadow-sm',
        !(submitted || isPendingQuestion) && 'border-blue-500/30 bg-blue-500/5',
      )}
    >
      <CardHeader className="pb-2">
        <CardTitle className="text-sm font-medium flex items-center gap-2">
          {submitted ? (
            <>
              <CheckCircle className="h-4 w-4 text-green-500" /> Questions Answered
            </>
          ) : (
            <>
              <HelpCircle className="h-4 w-4 text-blue-500" /> Claude needs your input
            </>
          )}
        </CardTitle>
      </CardHeader>
      <CardContent className="space-y-4">
        {questions.map((question) => (
          <div key={question.question} className="space-y-2">
            {question.header && <h4 className="text-sm font-semibold">{question.header}</h4>}
            <p className="text-sm text-muted-foreground">{question.question}</p>

            {question.multiSelect ? (
              <div className="space-y-2 pl-1">
                {question.options.map((opt) => {
                  const optId = `${question.question}-${opt.label}`
                  return (
                    <div key={opt.label} className="flex items-start gap-2">
                      <Checkbox
                        id={optId}
                        checked={answers[question.question]?.includes(opt.label) ?? false}
                        disabled={submitted}
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
                          <span className="text-muted-foreground ml-1">- {opt.description}</span>
                        )}
                      </Label>
                    </div>
                  )
                })}
              </div>
            ) : (
              <div className="space-y-2 pl-1">
                {question.options.map((opt) => {
                  const optId = `${question.question}-${opt.label}`
                  return (
                    <div key={opt.label} className="flex items-start gap-2">
                      <input
                        type="radio"
                        id={optId}
                        name={question.question}
                        checked={answers[question.question] === opt.label}
                        disabled={submitted}
                        onChange={() =>
                          setAnswers((prev) => ({ ...prev, [question.question]: opt.label }))
                        }
                        className="h-4 w-4 mt-0.5 accent-primary"
                      />
                      <Label htmlFor={optId} className="text-sm cursor-pointer">
                        <span className="font-medium">{opt.label}</span>
                        {opt.description && (
                          <span className="text-muted-foreground ml-1">- {opt.description}</span>
                        )}
                      </Label>
                    </div>
                  )
                })}
                <div className="flex items-start gap-2">
                  <input
                    type="radio"
                    id={`${question.question}-other`}
                    name={question.question}
                    checked={answers[question.question] === 'Other'}
                    disabled={submitted}
                    onChange={() =>
                      setAnswers((prev) => ({ ...prev, [question.question]: 'Other' }))
                    }
                    className="h-4 w-4 mt-0.5 accent-primary"
                  />
                  <div className="flex-1 space-y-1">
                    <Label htmlFor={`${question.question}-other`} className="text-sm font-medium">
                      Other
                    </Label>
                    {answers[question.question] === 'Other' && (
                      <Input
                        placeholder="Type your answer..."
                        value={otherText[question.question] || ''}
                        onChange={(e) =>
                          setOtherText((prev) => ({ ...prev, [question.question]: e.target.value }))
                        }
                        disabled={submitted}
                        className="h-8 text-sm"
                      />
                    )}
                  </div>
                </div>
              </div>
            )}
          </div>
        ))}

        {submitMutation.error && (
          <p className="text-sm text-destructive">
            {submitMutation.error instanceof Error
              ? submitMutation.error.message
              : 'Failed to submit'}
          </p>
        )}

        {!submitted && (
          <Button
            onClick={handleSubmit}
            disabled={!allAnswered || submitMutation.isPending}
            className="w-full"
            size="sm"
          >
            {submitMutation.isPending ? (
              <>
                <Loader2 className="mr-2 h-4 w-4 animate-spin" /> Submitting...
              </>
            ) : (
              'Submit Answers'
            )}
          </Button>
        )}
      </CardContent>
    </Card>
  )
}
