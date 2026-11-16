/**
 * "Connect tablet": starts the LAN server (`src-tauri/src/remote.rs`) and
 * shows its pairing QR code. Scanning it opens the UI in the tablet's
 * browser, where a pen draws on the page.
 */
import { backend, isDesktop, type RemoteInfo } from "../backend";
import { t } from "../i18n";
import { byId } from "./dom";
import { toast } from "./toast";

function show(info: RemoteInfo): void {
  const dialog = byId<HTMLDialogElement>("tablet-dialog");
  // the SVG comes from our own backend (qrcode crate), not from the network
  byId("tablet-qr").innerHTML = info.qrSvg;
  byId("tablet-url").textContent = info.url;
  if (!dialog.open) dialog.showModal();
}

async function connect(): Promise<void> {
  try {
    show(await backend.remoteStart());
  } catch (error) {
    toast(t("tablet.failed", { error: String(error) }), "error");
  }
}

export function initTablet(): void {
  const button = byId("btn-tablet");
  if (!isDesktop) {
    button.remove();
    return;
  }
  const dialog = byId<HTMLDialogElement>("tablet-dialog");
  button.addEventListener("click", () => {
    byId("connect-menu").classList.add("hidden");
    void connect();
  });
  byId("tablet-close").addEventListener("click", () => dialog.close());
  byId("tablet-stop").addEventListener("click", () => {
    void backend.remoteStop().then(() => toast(t("tablet.stopped"), "ok"));
    dialog.close();
  });
  dialog.addEventListener("click", (e) => {
    if (e.target === dialog) dialog.close(); // click on the backdrop
  });
}
