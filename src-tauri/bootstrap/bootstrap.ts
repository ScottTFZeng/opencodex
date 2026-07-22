import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

const title = document.getElementById("title")!;
const message = document.getElementById("message")!;
const actions = document.getElementById("actions")!;
const retry = document.getElementById("retry") as HTMLButtonElement;
const spinner = document.querySelector(".spinner")!;

function setRetryBusy(isBusy: boolean): void {
  retry.disabled = isBusy;
  retry.setAttribute("aria-busy", String(isBusy));
  retry.textContent = isBusy ? "Retrying…" : "Retry";
}

void listen<{ state: "ready" | "slow" | "failed"; url?: string }>("proxy-startup", ({ payload }) => {
  if (payload.state === "ready" && payload.url) {
    window.location.replace(payload.url);
    return;
  }
  setRetryBusy(false);
  actions.style.display = "flex";
  if (payload.state === "slow") {
    spinner.classList.remove("is-failed");
    title.textContent = "OpenCodex";
    message.textContent = "Still starting…";
  } else {
    spinner.classList.add("is-failed");
    title.textContent = "OpenCodex couldn't start";
    message.textContent = "Proxy unavailable.";
  }
});

void invoke("begin_startup");
retry.addEventListener("click", async () => {
  setRetryBusy(true);
  try {
    await invoke("retry_proxy");
  } catch {
    setRetryBusy(false);
  }
});
document.getElementById("logs")!.addEventListener("click", () => void invoke("open_logs"));
document.getElementById("exit")!.addEventListener("click", () => void invoke("exit_app"));
