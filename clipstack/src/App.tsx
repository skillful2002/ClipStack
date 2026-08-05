import { useState } from "react";

/**
 * P0 脚手架占位界面：仅用于验证「Tauri 窗口可正常拉起 + 设计 Token 可用」。
 * 真正的三栏主界面在阶段 P3 落地。
 */
export default function App() {
  const [count, setCount] = useState(0);

  return (
    <div className="boot">
      <div className="boot-logo">CS</div>
      <h1 className="boot-title">ClipStack</h1>
      <p className="boot-sub">剪切板管理 · 阶段 P0 脚手架已就绪</p>
      <button className="boot-btn" onClick={() => setCount((c) => c + 1)}>
        交互验证（点击计数）：{count}
      </button>
    </div>
  );
}
