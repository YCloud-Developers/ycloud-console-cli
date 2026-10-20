(() => {
  // The one-time code has already been delivered to the CLI. Keep it out of
  // the visible address and subsequent history navigation.
  try { history.replaceState(null, "", location.pathname); } catch {}
  const zh = navigator.language.toLowerCase().startsWith("zh");
  document.documentElement.lang = zh ? "zh" : "en";
  const copy = zh ? {
    received: ["浏览器授权已完成", "请返回终端，等待 YCloud CLI 提示登录成功。"],
    denied: ["你已取消授权", "本次授权已取消，YCloud CLI 不会通过此次请求获得访问权限。", "如需连接，请返回终端重新发起登录。"],
    rejected: ["无法验证本次回调", "本次请求未通过验证。请返回终端查看结果，或重新发起登录。", "如果终端仍在等待，请使用本次登录生成的授权链接。"],
    hint: "你可以手动关闭此标签页，返回发起请求的终端。"
  } : {
    received: ["Browser authorization complete", "Return to your terminal to confirm login."],
    denied: ["Authorization cancelled", "You cancelled this authorization. YCloud CLI will not gain access through this request.", "To connect, return to your terminal and start login again."],
    rejected: ["Unable to verify this callback", "This request could not be verified. Check your terminal or start login again.", "If your terminal is still waiting, use the authorization link from this login."],
    hint: "You can now close this tab manually and return to your terminal."
  };
  const state = document.body.dataset.state;
  const messages = copy[state] || copy.rejected;
  ["title", "description"].forEach((id, index) => {
    document.getElementById(id).textContent = messages[index];
  });
  document.title = messages[0] + " · YCloud";
  document.getElementById("symbol").textContent = state === "received" ? "✓" : "!";
  document.getElementById("hint").textContent = copy.hint;
})();
