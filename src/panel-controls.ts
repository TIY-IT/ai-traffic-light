// Closing the panel never deletes records or stops status collection.
// The list starts open on every launch so a previous dismiss cannot leave only a tiny lamp.
export function setupPanelControls() {
  const widget = document.querySelector<HTMLElement>(".widget");
  const close = document.querySelector<HTMLButtonElement>("#close-panel");
  if (!widget || !close) return;
  widget.dataset.panelDismissed = "false";
  widget.dataset.panelOpen = "true";
  const buttons: HTMLButtonElement[] = [];
  function setOpen(open: boolean) {
    widget!.dataset.panelDismissed = String(!open);
    widget!.dataset.panelOpen = String(open);
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
    button.setAttribute("aria-expanded", "true");
    button.addEventListener("pointerdown", event => event.stopPropagation());
    button.addEventListener("click", event => { event.stopPropagation(); setOpen(widget!.dataset.panelOpen !== "true"); });
    buttons.push(button);
    brand.replaceChildren(button);
  });
  close.addEventListener("click", event => { event.stopPropagation(); setOpen(false); buttons.find(b => !b.closest<HTMLElement>(".housing")?.hidden)?.focus(); });
  document.addEventListener("keydown", event => { if (event.key === "Escape") setOpen(false); });
}
