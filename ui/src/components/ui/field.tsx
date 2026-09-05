import * as React from "react";
import { useTranslation } from "react-i18next";

import { InfoHint } from "@/components/ui/info-hint";
import { cn } from "@/lib/utils";

// label + control + helper/error wrapper, mirrors the Rolter Design System
// Field. Composes around whatever control is passed as children (Input,
// Select, Textarea, Switch, ...).
export interface FieldProps extends React.HTMLAttributes<HTMLDivElement> {
  label?: string;
  /** explicit control id; when omitted the field generates one and applies it
   * to its child, so the label is never left dangling */
  htmlFor?: string;
  error?: string;
  hint?: string;
  // optional explanatory note surfaced via an (i) button beside the label:
  // what the field is and what values to use
  info?: React.ReactNode;
}

interface ControlProps {
  id?: string;
  "aria-describedby"?: string;
  "aria-invalid"?: boolean | "true" | "false";
}

export function Field({
  label,
  htmlFor,
  error,
  hint,
  info,
  className,
  children,
  ...props
}: FieldProps) {
  // almost no caller passes `htmlFor`, which left every label in the dashboard
  // pointing at nothing: a screen reader announced the control as unlabelled,
  // and `getByLabelText` could not find it either. Generate the id here and put
  // it on the child, so the association is the default rather than something
  // each of ~200 call sites has to remember.
  const { t } = useTranslation();
  const generated = React.useId();
  const messageId = React.useId();
  const single = React.Children.count(children) === 1 ? children : null;
  const control = React.isValidElement<ControlProps>(single) ? single : null;
  const controlId = htmlFor ?? control?.props.id ?? (control ? generated : undefined);
  // the error or hint below the control is tied to it as its description, and
  // an error also flips aria-invalid, so a screen reader hears both the state
  // and the reason rather than a control that silently refuses to submit
  const message = error ?? hint;
  const described: ControlProps = {};
  if (control && !control.props.id && !htmlFor) described.id = controlId;
  if (message && control && !control.props["aria-describedby"]) {
    described["aria-describedby"] = messageId;
  }
  if (error && control && control.props["aria-invalid"] === undefined) {
    described["aria-invalid"] = true;
  }
  const labelled =
    control && Object.keys(described).length > 0
      ? React.cloneElement(control, described)
      : children;

  return (
    <div className={cn("space-y-1.5", className)} {...props}>
      {label && (
        <div className="flex items-center gap-1.5">
          <label htmlFor={controlId} className="text-sm font-medium leading-none">
            {label}
          </label>
          {info && <InfoHint text={info} label={t("common.aboutField", { label })} />}
        </div>
      )}
      {labelled}
      {error ? (
        <p id={messageId} role="alert" className="text-xs text-[color:var(--status-danger-text)]">
          {error}
        </p>
      ) : hint ? (
        <p id={messageId} className="text-xs text-muted-foreground">
          {hint}
        </p>
      ) : null}
    </div>
  );
}
