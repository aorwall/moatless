import * as React from 'react';

interface FormProps extends React.HTMLAttributes<HTMLFormElement> {}

const Form = React.forwardRef<HTMLFormElement, FormProps>(
  ({ className, ...props }, ref) => {
    return (
      <form
        ref={ref}
        className={className}
        {...props}
      />
    );
  }
);

Form.displayName = 'Form';

interface FormFieldProps extends React.HTMLAttributes<HTMLDivElement> {}

const FormField = React.forwardRef<HTMLDivElement, FormFieldProps>(
  ({ className, ...props }, ref) => {
    return (
      <div
        ref={ref}
        className={`space-y-2 ${className || ''}`}
        {...props}
      />
    );
  }
);

FormField.displayName = 'FormField';

interface FormLabelProps extends React.LabelHTMLAttributes<HTMLLabelElement> {}

const FormLabel = React.forwardRef<HTMLLabelElement, FormLabelProps>(
  ({ className, ...props }, ref) => {
    return (
      <label
        ref={ref}
        className={`text-sm font-medium leading-none peer-disabled:cursor-not-allowed peer-disabled:opacity-70 ${className || ''}`}
        {...props}
      />
    );
  }
);

FormLabel.displayName = 'FormLabel';

interface FormDescriptionProps extends React.HTMLAttributes<HTMLParagraphElement> {}

const FormDescription = React.forwardRef<HTMLParagraphElement, FormDescriptionProps>(
  ({ className, ...props }, ref) => {
    return (
      <p
        ref={ref}
        className={`text-sm text-muted-foreground ${className || ''}`}
        {...props}
      />
    );
  }
);

FormDescription.displayName = 'FormDescription';

interface FormMessageProps extends React.HTMLAttributes<HTMLParagraphElement> {}

const FormMessage = React.forwardRef<HTMLParagraphElement, FormMessageProps>(
  ({ className, ...props }, ref) => {
    return (
      <p
        ref={ref}
        className={`text-sm font-medium text-destructive ${className || ''}`}
        {...props}
      />
    );
  }
);

FormMessage.displayName = 'FormMessage';

export {
    Form, FormDescription, FormField,
    FormLabel, FormMessage
};
