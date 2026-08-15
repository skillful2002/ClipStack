// 轻量悬浮提示：hover/focus 元素时显示气泡，渲染到 body 以避免被滚动容器裁剪。

import { useState, useRef, type ReactNode } from "react";
import { createPortal } from "react-dom";
import "./Tooltip.css";

interface Props {
  label: ReactNode;
  children: ReactNode;
  /** 显示延迟（毫秒），避免鼠标快速划过时闪烁 */
  delay?: number;
}

export function Tooltip({ label, children, delay = 200 }: Props) {
  const [pos, setPos] = useState<{ x: number; y: number } | null>(null);
  const anchorRef = useRef<HTMLSpanElement>(null);
  const timer = useRef<number | null>(null);

  const show = () => {
    if (timer.current) window.clearTimeout(timer.current);
    timer.current = window.setTimeout(() => {
      const el = anchorRef.current;
      if (!el) return;
      const r = el.getBoundingClientRect();
      setPos({ x: r.left + r.width / 2, y: r.top });
    }, delay);
  };

  const hide = () => {
    if (timer.current) window.clearTimeout(timer.current);
    setPos(null);
  };

  return (
    <span
      ref={anchorRef}
      className="tooltip-anchor"
      onMouseEnter={show}
      onMouseLeave={hide}
      onFocus={show}
      onBlur={hide}
    >
      {children}
      {pos &&
        createPortal(
          <span className="tooltip-bubble" style={{ left: pos.x, top: pos.y - 8 }} role="tooltip">
            {label}
          </span>,
          document.body,
        )}
    </span>
  );
}
