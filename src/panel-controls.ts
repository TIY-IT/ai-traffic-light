// Closing the panel never deletes records or stops status collection.
export function setupPanelControls() {
  const widget = document.querySelector<HTMLElement>(".widget");
  const close = document.querySelector<HTMLButtonElement>("#close-panel");
  if (!widget || !close) return;
  const key = "ai-light-panel-dismissed";
  try { widget.dataset.panelDismissed = String(localStorage.getItem(key) === "true"); } catch { /* Storage may be unavailable. */ }
  const buttons: HTMLButtonElement[] = [];
  function setOpen(open: boolean) {
    widget!.dataset.panelDismissed = "true";
    widget!.dataset.panelOpen = String(open);
    try { localStorage.setItem(key, "true"); } catch { /* Keep in-memory state. */ }
    for (const button of buttons) button.setAttribute("aria-expanded", String(open));
    document.dispatchEvent(new CustomEvent("hit-regions-dirty"));
  }
  document.querySelectorAll<HTMLElement>(".brand").forEach(brand => {
    brand.removeAttribute("data-tauri-drag-region");
    const button = document.createElement("button");
    button.type = "button";
    button.className = "details-toggle";
    button.textContent = "≡";
    button.title = "查看 / 收起会话";
    button.setAttribute("aria-label", "查看 / 收起会话面板");
    button.setAttribute("aria-expanded", "false");
    button.addEventListener("pointerdown", event => event.stopPropagation());
    button.addEventListener("click", event => { event.stopPropagation(); setOpen(widget!.dataset.panelOpen !== "true"); });
    buttons.push(button);
    brand.replaceChildren(button);
  });
  close.addEventListener("click", event => { event.stopPropagation(); setOpen(false); buttons.find(b => !b.closest<HTMLElement>(".housing")?.hidden)?.focus(); });
  document.addEventListener("keydown", event => { if (event.key === "Escape") setOpen(false); });
}
