import { useState, type InputHTMLAttributes } from 'react';

/** Preserve the user's decimal text while editing. Formatting every keystroke
 * (e.g. 0. -> 0.000) moves the caret and makes decimal boundaries impossible. */
export function TimeInput({ value, onChange, onBlur, onKeyDown, ...props }: InputHTMLAttributes<HTMLInputElement>) {
  const [draft, setDraft] = useState<string | null>(null);
  return <input {...props} value={draft ?? value} onChange={event => {
    setDraft(event.target.value);
    onChange?.(event);
  }} onBlur={event => {
    setDraft(null);
    onBlur?.(event);
  }} onKeyDown={event => {
    onKeyDown?.(event);
    if (event.key === 'Enter') {
      event.preventDefault();
      event.stopPropagation();
      event.currentTarget.blur();
    }
  }} />;
}
