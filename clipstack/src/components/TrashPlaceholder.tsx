// P3 · 回收站占位（删除的条目管理将在后续阶段提供）。

export function TrashPlaceholder() {
  return (
    <section className="settings-pane">
      <h2 className="settings-title">回收站</h2>
      <div className="trash-placeholder">
        <div className="trash-placeholder-icon">🗑️</div>
        <p>已删除的剪贴板记录会暂存于此，支持恢复与彻底清理。</p>
        <p className="settings-note">该模块将在后续版本提供。</p>
      </div>
    </section>
  );
}
