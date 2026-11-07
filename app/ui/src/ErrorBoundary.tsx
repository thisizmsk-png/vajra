import { Component } from "react";
import type { ReactNode } from "react";
import { Button, Text } from "../components/ui/base";
import { S } from "../strings";

/** A screen that throws shows this instead of a blank window; the rest of the shell keeps working. */
export class ErrorBoundary extends Component<{ children: ReactNode; resetKey?: string }, { failed: boolean }> {
  state = { failed: false };
  static getDerivedStateFromError() {
    return { failed: true };
  }
  componentDidUpdate(prev: { resetKey?: string }) {
    if (this.state.failed && prev.resetKey !== this.props.resetKey) this.setState({ failed: false });
  }
  render() {
    if (!this.state.failed) return this.props.children;
    return (
      <div role="alert" className="v-shell__error">
        <Text as="h2" variant="headline-md">{S.shell.errorTitle}</Text>
        <Text variant="body-sm">{S.shell.errorBody}</Text>
        <Button onClick={() => this.setState({ failed: false })}>{S.shell.retry}</Button>
      </div>
    );
  }
}
