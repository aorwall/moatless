import { AlertCircle } from 'lucide-react';

import { getErrorMessage } from '@/lib/utils/error';

import { Alert, AlertDescription, AlertTitle } from './alert';

interface ErrorAlertProps {
  error: unknown;
  title?: string;
  className?: string;
}

export function ErrorAlert({
  error,
  title = 'Error',
  className,
}: ErrorAlertProps) {
  const message = getErrorMessage(error);

  return (
    <Alert variant="destructive" className={className}>
      <AlertCircle className="h-4 w-4" />
      <AlertTitle>{title}</AlertTitle>
      <AlertDescription>{message}</AlertDescription>
    </Alert>
  );
}