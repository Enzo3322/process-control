import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import "./styles.css";

type ProcessInfo = {
  port: number;
  pid: number;
  name: string;
  protocol: string;
  address: string;
};

const appRoot = document.querySelector<HTMLDivElement>("#app");

if (!appRoot) {
  throw new Error("App root not found");
}

const app = appRoot;

const COMMON_PORTS = [80, 3000, 3001, 3333, 4000, 4200, 4321, 5000, 5173, 5432, 6379, 8000, 8080, 8081, 8888];

let processes: ProcessInfo[] = [];
let isLoading = false;
let actionPid: number | null = null;
let errorMessage = "";

const portLabel = (port: number) => {
  const labels: Record<number, string> = {
    80: "HTTP",
    3000: "Frontend",
    3001: "Alt Frontend",
    3333: "API",
    4000: "Backend",
    4200: "Angular",
    4321: "Wasp",
    5000: "Flask/Rails",
    5173: "Vite",
    5432: "Postgres",
    6379: "Redis",
    8000: "Dev Server",
    8080: "Proxy",
    8081: "Alt Proxy",
    8888: "Jupyter"
  };

  return labels[port] ?? "Common Port";
};

function render() {
  const groupedPorts = COMMON_PORTS.map((port) => ({
    port,
    items: processes.filter((process) => process.port === port)
  }));

  app.innerHTML = `
    <main class="shell">
      <section class="panel">
        <div class="panel__glow"></div>
        <header class="hero">
          <div>
            <p class="eyebrow">Menu Bar Monitor</p>
            <h1>Process Control</h1>
          </div>
          <button class="refresh-button" data-action="refresh" ${isLoading ? "disabled" : ""}>
            ${isLoading ? "Atualizando..." : "Atualizar"}
          </button>
        </header>

        <p class="description">
          Acompanhe as portas mais comuns do seu ambiente de desenvolvimento e encerre processos presos sem sair da menu bar.
        </p>

        ${
          errorMessage
            ? `<div class="message message--error">${errorMessage}</div>`
            : ""
        }

        <div class="process-list">
          ${groupedPorts
            .map(({ port, items }) => {
              const hasProcess = items.length > 0;

              return `
                <article class="port-card ${hasProcess ? "port-card--active" : ""}">
                  <div class="port-card__header">
                    <div>
                      <span class="port-chip">:${port}</span>
                      <h2>${portLabel(port)}</h2>
                    </div>
                    <span class="status ${hasProcess ? "status--live" : ""}">
                      ${hasProcess ? `${items.length} ativo${items.length > 1 ? "s" : ""}` : "Livre"}
                    </span>
                  </div>

                  ${
                    hasProcess
                      ? items
                          .map(
                            (process) => `
                              <div class="process-row">
                                <div>
                                  <strong>${process.name}</strong>
                                  <p>PID ${process.pid} • ${process.protocol} • ${process.address}</p>
                                </div>
                                <button
                                  class="kill-button"
                                  data-action="kill"
                                  data-pid="${process.pid}"
                                  ${actionPid === process.pid ? "disabled" : ""}
                                >
                                  ${actionPid === process.pid ? "Encerrando..." : "Matar"}
                                </button>
                              </div>
                            `
                          )
                          .join("")
                      : `<p class="empty-state">Nenhum processo escutando nessa porta.</p>`
                  }
                </article>
              `;
            })
            .join("")}
        </div>
      </section>
    </main>
  `;

  app.querySelector<HTMLButtonElement>("[data-action='refresh']")?.addEventListener("click", () => {
    void refreshProcesses();
  });

  app.querySelectorAll<HTMLButtonElement>("[data-action='kill']").forEach((button) => {
    button.addEventListener("click", () => {
      const pid = Number(button.dataset.pid);
      if (!Number.isNaN(pid)) {
        void killProcess(pid);
      }
    });
  });
}

async function refreshProcesses() {
  isLoading = true;
  errorMessage = "";
  render();

  try {
    processes = await invoke<ProcessInfo[]>("list_common_port_processes");
  } catch (error) {
    errorMessage = error instanceof Error ? error.message : "Nao foi possivel listar os processos.";
  } finally {
    isLoading = false;
    render();
  }
}

async function killProcess(pid: number) {
  actionPid = pid;
  errorMessage = "";
  render();

  try {
    await invoke("kill_process", { pid });
    await refreshProcesses();
  } catch (error) {
    errorMessage = error instanceof Error ? error.message : `Nao foi possivel encerrar o PID ${pid}.`;
  } finally {
    actionPid = null;
    render();
  }
}

render();
void refreshProcesses();
void listen<ProcessInfo[]>("ports-updated", (event) => {
  processes = event.payload;
  errorMessage = "";
  isLoading = false;
  actionPid = null;
  render();
});

window.setInterval(() => {
  void refreshProcesses();
}, 7000);
