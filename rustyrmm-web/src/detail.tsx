import { Fragment } from "preact";
import { useEffect, useState } from "preact/hooks";
import { timestampDate, timestampFromDate, type Timestamp } from "@bufbuild/protobuf/wkt";
import { Interpreter, type ProcessInfo, type Sample } from "./gen/rustyrmm/common/v1/common_pb";
import { CommandStatus, type CommandRecord, type Endpoint, type Environment } from "./gen/rustyrmm/operator/v1/operator_pb";
import { all, api, message, needsLogin, THRESHOLD_ERROR, thresholdMinutes, when } from "./api";

const RANGES: [number, string][] = [
  [3600, "1 hour"],
  [86400, "24 hours"],
  [7 * 86400, "7 days"],
  [30 * 86400, "30 days"],
];

const INTERPRETERS: [Interpreter, string][] = [
  [Interpreter.SH, "sh"],
  [Interpreter.BASH, "bash"],
  [Interpreter.POWERSHELL, "PowerShell"],
  [Interpreter.CMD, "cmd"],
];

function bytes(n: number | bigint): string {
  let v = Number(n);
  for (const unit of ["B", "KB", "MB", "GB", "TB"]) {
    if (v < 1024 || unit === "TB") return `${v.toFixed(unit === "B" ? 0 : 1)} ${unit}`;
    v /= 1024;
  }
  return "";
}
const rate = (n: number) => `${bytes(n)}/s`;
const pct = (n: number) => `${n.toFixed(0)}%`;
const ms = (ts: Timestamp | undefined) => (ts ? timestampDate(ts).getTime() : 0);

export function EndpointDetail({ id, onSessionEnded }: { id: string; onSessionEnded: () => void }) {
  const [endpoint, setEndpoint] = useState<Endpoint | undefined>(undefined);
  const [error, setError] = useState("");

  const failed = (e: unknown) => {
    if (needsLogin(e)) onSessionEnded();
    else setError(message(e));
  };

  const load = () => {
    api
      .getEndpoint({ id })
      .then((ep) => {
        setEndpoint(ep);
        setError("");
      })
      .catch(failed);
  };

  useEffect(() => {
    load();
    const timer = setInterval(load, 30_000);
    return () => {
      clearInterval(timer);
    };
  }, [id]);

  const revoke = () => {
    if (!confirm("Revoke this Endpoint? Its Agent is disconnected and can't reconnect.")) return;
    api.revokeEndpoint({ id }).then(load).catch(failed);
  };
  const remove = () => {
    if (!confirm("Delete this Endpoint and its Samples and Inventory? Command history is kept.")) return;
    api
      .deleteEndpoint({ id })
      .then(() => {
        location.hash = "";
      })
      .catch(failed);
  };

  if (!endpoint) {
    return (
      <main>
        <a href="#">← Endpoints</a>
        {error ? <p class="error" role="alert">{error}</p> : <p class="muted">Loading…</p>}
      </main>
    );
  }

  return (
    <main>
      <a href="#">← Endpoints</a>
      <div class="toolbar">
        <h2>
          {endpoint.hostname || "(hostname not reported yet)"}{" "}
          <span class={endpoint.revoked ? "badge revoked" : endpoint.online ? "badge online" : "badge offline"}>
            {endpoint.revoked ? "Revoked" : endpoint.online ? "Online" : "Offline"}
          </span>
        </h2>
        <div class="actions">
          {!endpoint.revoked && <button onClick={revoke}>Revoke</button>}
          <button class="danger" onClick={remove}>
            Delete
          </button>
        </div>
      </div>
      {error && <p class="error" role="alert">{error}</p>}
      <AlertSettings key={endpoint.id} endpoint={endpoint} saved={load} failed={failed} />
      <Inventory endpoint={endpoint} />
      <Charts id={id} failed={failed} />
      <Processes id={id} failed={failed} />
      <Commands endpoint={endpoint} failed={failed} />
    </main>
  );
}

function AlertSettings({ endpoint, saved, failed }: { endpoint: Endpoint; saved: () => void; failed: (e: unknown) => void }) {
  const [enabled, setEnabled] = useState(endpoint.alertsEnabled);
  const [threshold, setThreshold] = useState(endpoint.offlineThresholdMinutes?.toString() ?? "");
  const [invalid, setInvalid] = useState(false);
  const [environment, setEnvironment] = useState<Environment | undefined>(undefined);

  useEffect(() => {
    all(async (pageToken) => {
      const r = await api.listEnvironments({ pageToken });
      return { items: r.environments, next: r.nextPageToken };
    })
      .then((list) => {
        setEnvironment(list.find((e) => e.id === endpoint.environmentId));
      })
      .catch(failed);
  }, [endpoint.environmentId]);

  const save = (e: Event) => {
    e.preventDefault();
    const blank = threshold.trim() === "";
    const offlineThresholdMinutes = blank ? undefined : thresholdMinutes(threshold);
    setInvalid(!blank && offlineThresholdMinutes === undefined);
    if (!blank && offlineThresholdMinutes === undefined) return;
    api.setEndpointAlertSettings({ endpointId: endpoint.id, alertsEnabled: enabled, offlineThresholdMinutes }).then(saved).catch(failed);
  };
  const effective = endpoint.offlineThresholdMinutes ?? environment?.offlineThresholdMinutes;
  return (
    <section>
      <h3>Alerts</h3>
      <form class="inline" onSubmit={save}>
        <label>
          <input
            type="checkbox"
            checked={enabled}
            onInput={(e) => {
              setEnabled(e.currentTarget.checked);
            }}
          />{" "}
          Alerts on
        </label>
        <label>
          Offline threshold (minutes){" "}
          <input
            inputMode="numeric"
            placeholder="Environment default"
            value={threshold}
            onInput={(e) => {
              setThreshold(e.currentTarget.value);
            }}
          />
        </label>
        <button type="submit">Save</button>
      </form>
      {invalid && (
        <p class="error" role="alert">
          {THRESHOLD_ERROR}
        </p>
      )}
      {effective !== undefined && <p class="muted">Alerts after about {effective} minutes offline.</p>}
    </section>
  );
}

function Inventory({ endpoint }: { endpoint: Endpoint }) {
  const inv = endpoint.inventory;
  const rows: [string, string][] = [
    ["Paired", when(endpoint.pairedAt)],
    ["Last seen", endpoint.online ? "now" : when(endpoint.lastSeenAt)],
  ];
  if (inv) {
    const os = inv.os;
    const cpu = inv.cpu;
    const updates = inv.updates;
    const count = (n: number | undefined) => (n === undefined ? "unknown" : String(n));
    rows.push(
      ["OS", os ? `${os.name} ${os.version}`.trim() : ""],
      ["Virtualization", os?.virtSystem === "none" ? "none (physical)" : [os?.virtSystem, os?.virtRole].filter(Boolean).join(" ")],
      ["CPU", cpu ? `${cpu.brand} · ${cpu.coreCount} cores / ${cpu.threadCount} threads` : ""],
      ["Memory", bytes(inv.memoryTotalBytes)],
      ["Disks", inv.disks.map((d) => `${d.mountPoint || d.name} ${bytes(d.totalBytes)}`).join(", ")],
      ["Network", inv.interfaces.map((i) => `${i.name} ${[...i.ipv4, ...i.ipv6].join(" ")}`.trim()).join("; ")],
      ["Pending updates", updates ? `${count(updates.regular)} regular, ${count(updates.security)} security` : "unknown"],
      ["Timezone", inv.timezone],
      ["System UUID", inv.systemUuid],
      ["Agent version", inv.agentVersion],
      ["Inventory collected", when(inv.collectedAt)],
    );
  }
  return (
    <section>
      <h3>Inventory</h3>
      {!inv && <p class="muted">The Agent hasn't sent an Inventory yet.</p>}
      <dl class="inventory">
        {rows.map(([k, v]) => (
          <Fragment key={k}>
            <dt>{k}</dt>
            <dd>{v || <span class="muted">unknown</span>}</dd>
          </Fragment>
        ))}
      </dl>
    </section>
  );
}

function Charts({ id, failed }: { id: string; failed: (e: unknown) => void }) {
  const [range, setRange] = useState(3600); // Seconds.
  const [data, setData] = useState<{ from: number; to: number; samples: Sample[] } | undefined>(undefined);

  useEffect(() => {
    const load = () => {
      const to = Date.now();
      const from = to - range * 1000;
      void all(async (pageToken) => {
        const r = await api.listSamples({
          endpointId: id,
          start: timestampFromDate(new Date(from)),
          end: timestampFromDate(new Date(to)),
          pageSize: 1000,
          pageToken,
        });
        return { items: r.samples, next: r.nextPageToken };
      })
        .then((samples) => {
          setData({ from, to, samples });
        })
        .catch(failed);
    };
    load();
    const timer = setInterval(load, 30_000);
    return () => {
      clearInterval(timer);
    };
  }, [id, range]);

  const samples = data?.samples ?? [];
  const line = (f: (s: Sample) => number) => samples.map((s): [number, number] => [ms(s.collectedAt), f(s)]);
  const sum = <T,>(f: (s: Sample) => T[], g: (x: T) => bigint) => line((s) => f(s).reduce((a, x) => a + Number(g(x)), 0));
  const props = { from: data?.from ?? 0, to: data?.to ?? 1 };

  return (
    <section>
      <div class="toolbar">
        <h3>Monitoring</h3>
        <label>
          Range{" "}
          <select
            value={range}
            onChange={(e) => {
              setRange(Number(e.currentTarget.value));
            }}
          >
            {RANGES.map(([secs, label]) => (
              <option key={secs} value={secs}>
                {label}
              </option>
            ))}
          </select>
        </label>
      </div>
      {data === undefined ? (
        <p class="muted">Loading…</p>
      ) : samples.length === 0 ? (
        <p class="muted">No Samples in this range.</p>
      ) : (
        <div class="charts">
          <Chart {...props} title="CPU" max={100} format={pct} series={[{ label: "used", points: line((s) => s.cpuUsagePercent) }]} />
          <Chart
            {...props}
            title="Memory"
            max={100}
            format={pct}
            series={[
              {
                label: "used",
                points: line((s) => (s.memoryTotalBytes ? (Number(s.memoryUsedBytes) / Number(s.memoryTotalBytes)) * 100 : 0)),
              },
            ]}
          />
          <Chart
            {...props}
            title="Disk"
            format={rate}
            series={[
              { label: "read", points: sum((s) => s.disks, (d) => d.readBytesPerSec) },
              { label: "write", points: sum((s) => s.disks, (d) => d.writeBytesPerSec) },
            ]}
          />
          <Chart
            {...props}
            title="Network"
            format={rate}
            series={[
              { label: "in", points: sum((s) => s.interfaces, (i) => i.rxBytesPerSec) },
              { label: "out", points: sum((s) => s.interfaces, (i) => i.txBytesPerSec) },
            ]}
          />
        </div>
      )}
    </section>
  );
}

const W = 600;
const H = 120;

// shortcut: lines join straight across Offline gaps; break them if that misleads Operators.
function Chart(p: {
  title: string;
  from: number;
  to: number;
  series: { label: string; points: [number, number][] }[]; // [time in ms, value]
  max?: number;
  format: (v: number) => string;
}) {
  const top = p.max ?? Math.max(1, ...p.series.flatMap((s) => s.points.map(([, v]) => v)));
  const x = (t: number) => (((t - p.from) / (p.to - p.from)) * W).toFixed(1);
  const y = (v: number) => (H - (Math.min(v, top) / top) * H).toFixed(1);
  return (
    <figure class="chart">
      <figcaption>
        <strong>{p.title}</strong>{" "}
        {p.series.map((s, i) => {
          const last = s.points.at(-1)?.[1];
          return (
            <Fragment key={s.label}>
              {" "}
              <span class={`key key${i}`}>
                {s.label} {last === undefined ? "–" : p.format(last)}
              </span>
            </Fragment>
          );
        })}
        <span class="muted"> · scale {p.format(top)}</span>
      </figcaption>
      <svg viewBox={`0 0 ${W} ${H}`} preserveAspectRatio="none" role="img" aria-label={`${p.title} chart`}>
        {p.series.map((s, i) => (
          <polyline key={s.label} class={`line key${i}`} points={s.points.map(([t, v]) => `${x(t)},${y(v)}`).join(" ")} />
        ))}
      </svg>
    </figure>
  );
}

function Processes({ id, failed }: { id: string; failed: (e: unknown) => void }) {
  const [result, setResult] = useState<{ list: ProcessInfo[]; at: Timestamp | undefined } | undefined>(undefined);
  const [busy, setBusy] = useState(false);

  const fetchList = () => {
    setBusy(true);
    api
      .getProcessList({ endpointId: id })
      .then((r) => {
        setResult({ list: [...r.processes].sort((a, b) => b.cpuUsagePercent - a.cpuUsagePercent), at: r.receivedAt });
      })
      .catch(failed)
      .finally(() => {
        setBusy(false);
      });
  };

  return (
    <section>
      <div class="toolbar">
        <h3>Processes</h3>
        <button onClick={fetchList} disabled={busy}>
          {busy ? "Asking the Agent…" : result ? "Refresh" : "Get processes"}
        </button>
      </div>
      {result && (
        <>
          <p class="muted">
            {result.list.length} processes as of {when(result.at)}
          </p>
          <div class="scroll">
            <table>
              <thead>
                <tr>
                  <th>PID</th>
                  <th>Name</th>
                  <th>CPU</th>
                  <th>Memory</th>
                  <th>Disk read/write</th>
                  <th>Command line</th>
                </tr>
              </thead>
              <tbody>
                {result.list.map((p) => (
                  <tr key={p.pid}>
                    <td>{p.pid}</td>
                    <td>{p.name}</td>
                    <td>{pct(p.cpuUsagePercent)}</td>
                    <td>{bytes(p.memoryBytes)}</td>
                    <td>
                      {rate(Number(p.diskReadBytesPerSec))} / {rate(Number(p.diskWriteBytesPerSec))}
                    </td>
                    <td class="cmdline">{p.commandLine.join(" ")}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </>
      )}
    </section>
  );
}

const decoder = new TextDecoder(); // Output may not be UTF-8; invalid bytes show as U+FFFD.

function outcome(c: CommandRecord): string {
  if (c.status === CommandStatus.DISPATCHED) return "Running…";
  if (c.status === CommandStatus.OUTCOME_UNKNOWN) return "Outcome unknown";
  const r = c.result;
  if (!r) return "Completed";
  if (r.timedOut) return "Timed out";
  return r.exitCode === undefined ? "Killed" : `Exit ${r.exitCode}`;
}

function Commands({ endpoint, failed }: { endpoint: Endpoint; failed: (e: unknown) => void }) {
  const windows = endpoint.inventory?.os?.family === "windows";
  const [interpreter, setInterpreter] = useState<Interpreter | undefined>(undefined);
  const [script, setScript] = useState("");
  const [timeout, setTimeoutSeconds] = useState(300);
  const [busy, setBusy] = useState(false);
  const [history, setHistory] = useState<CommandRecord[] | undefined>(undefined);
  const [runs, setRuns] = useState(0); // Bumped after RunCommand to reload the history at once.

  const running = history?.some((c) => c.status === CommandStatus.DISPATCHED) ?? false;

  useEffect(() => {
    const load = () => {
      // shortcut: newest 100 only; page through ListCommands if Operators need older history here.
      api
        .listCommands({ endpointId: endpoint.id, pageSize: 100 })
        .then((r) => {
          setHistory(r.commands);
        })
        .catch(failed);
    };
    load();
    const timer = setInterval(load, running ? 3000 : 30_000); // Poll fast while a Command is running.
    return () => {
      clearInterval(timer);
    };
  }, [endpoint.id, running, runs]);

  async function submit(e: Event) {
    e.preventDefault();
    setBusy(true);
    try {
      await api.runCommand({
        endpointId: endpoint.id,
        interpreter: interpreter ?? (windows ? Interpreter.POWERSHELL : Interpreter.SH),
        script,
        timeoutSeconds: timeout,
      });
      setScript("");
      setRuns((n) => n + 1);
    } catch (err) {
      failed(err);
    } finally {
      setBusy(false);
    }
  }

  return (
    <section>
      <h3>Run a Command</h3>
      <form
        class="command"
        onSubmit={(e) => {
          void submit(e);
        }}
      >
        <div class="toolbar">
          <label>
            Interpreter{" "}
            <select
              value={interpreter ?? (windows ? Interpreter.POWERSHELL : Interpreter.SH)}
              onChange={(e) => {
                setInterpreter(Number(e.currentTarget.value) as Interpreter);
              }}
            >
              {INTERPRETERS.map(([v, label]) => (
                <option key={v} value={v}>
                  {label}
                </option>
              ))}
            </select>
          </label>
          <label>
            Timeout (seconds){" "}
            <input
              type="number"
              min={1}
              max={3600}
              value={timeout}
              onInput={(e) => {
                setTimeoutSeconds(Number(e.currentTarget.value));
              }}
              required
            />
          </label>
        </div>
        <textarea
          aria-label="Script"
          rows={5}
          value={script}
          onInput={(e) => {
            setScript(e.currentTarget.value);
          }}
          required
        />
        <div>
          <button type="submit" disabled={busy || endpoint.revoked || !endpoint.online}>
            {busy ? "Sending…" : "Run"}
          </button>{" "}
          {endpoint.revoked ? (
            <span class="muted">The Endpoint is Revoked.</span>
          ) : (
            !endpoint.online && <span class="muted">The Endpoint is Offline; Commands are never queued.</span>
          )}
        </div>
      </form>

      <h3>Command history</h3>
      {history === undefined ? (
        <p class="muted">Loading…</p>
      ) : history.length === 0 ? (
        <p class="muted">No Commands yet.</p>
      ) : (
        history.map((c) => (
          <details key={c.id} class="history">
            <summary>
              {when(c.createdAt)} · {c.operatorUsername} · {INTERPRETERS.find(([v]) => v === c.interpreter)?.[1] ?? "?"} ·{" "}
              <strong>{outcome(c)}</strong>
            </summary>
            <pre>{c.script}</pre>
            {c.result && (
              <>
                <h4>stdout</h4>
                <pre>{decoder.decode(c.result.stdout)}</pre>
                <h4>stderr</h4>
                <pre>{decoder.decode(c.result.stderr)}</pre>
                {c.result.truncated && <p class="muted">Output was truncated by the Agent.</p>}
              </>
            )}
          </details>
        ))
      )}
    </section>
  );
}
