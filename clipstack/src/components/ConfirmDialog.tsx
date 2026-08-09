// 通用确认对话框（模态）。
//
// Tauri 2 的 WebView 默认不支持原生 window.confirm（调用静默失败/直接返回 false），
// 因此所有危险操作的确认必须使用本组件。样式复用 HistoryList「清除全部」的
// confirm-* 系列类（深浅色自适应）。
//
// 用法：open 为 true 时渲染遮罩 + 对话框；onConfirm / onCancel 分别处理确认与取消；
// danger 使确认按钮呈红色（危险操作）；busy 时禁用两个按钮（异步处理中防重复点击）。

interface ConfirmDialogProps {
  open: boolean;
  title: string;
  message: string;
  confirmLabel: string;
  cancelLabel: string;
  danger?: boolean;
  busy?: boolean;
  onConfirm: () => void;
  onCancel: () => void;
}

export function ConfirmDialog({
  open,
  title,
  message,
  confirmLabel,
  cancelLabel,
  danger,
  busy,
  onConfirm,
  onCancel,
}: ConfirmDialogProps) {
  if (!open) return null;
  return (
    <div className="confirm-overlay" role="dialog" aria-modal="true" onClick={onCancel}>
      <div className="confirm-dialog" onClick={(e) => e.stopPropagation()}>
        <div className="confirm-title">{title}</div>
        <div className="confirm-body">{message}</div>
        <div className="confirm-actions">
          <button className="confirm-btn cancel" onClick={onCancel} disabled={busy}>
            {cancelLabel}
          </button>
          <button
            className={`confirm-btn${danger ? " danger" : ""}`}
            onClick={onConfirm}
            disabled={busy}
          >
            {confirmLabel}
          </button>
        </div>
      </div>
    </div>
  );
}
