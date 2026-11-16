import { Component, type ErrorInfo, type ReactNode } from "react";
import { QueryError } from "./QueryError";

interface Props {
  children: ReactNode;
  /** 出错区域的名字，只显示给用户看，不参与逻辑。 */
  zone?: string;
}

interface State {
  error: Error | null;
}

/**
 * 页面级错误边界。没有它的时候，单个页面渲染抛错会把整棵 React 树卸掉——
 * 用户看到的就是「窗口一片黑」（真出事过一次：钩子写在提前 return 之后）。
 * 现在降级成一张错误卡片，其余页面照常可用。
 */
export class ErrorBoundary extends Component<Props, State> {
  state: State = { error: null };

  static getDerivedStateFromError(error: Error): State {
    return { error };
  }

  componentDidCatch(error: Error, info: ErrorInfo) {
    console.error(`[ui:${this.props.zone ?? "page"}]`, error, info.componentStack);
  }

  render() {
    if (!this.state.error) return this.props.children;
    // 与 QueryError 同一张红卡：页面级崩溃和读取失败长得一样，用户不必学两套。
    return (
      <QueryError
        title={`${this.props.zone ?? "这一页"}渲染出错了`}
        error={this.state.error}
        retryLabel="重试这一页"
        onRetry={() => this.setState({ error: null })}
      />
    );
  }
}
