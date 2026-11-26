import { useEffect, useRef, useState } from "preact/hooks";
import { timestampFromDate } from "@bufbuild/protobuf/wkt";
import { AlertSeverity, type Alert, type Environment } from "./gen/rustyrmm/operator/v1/operator_pb";
import { all, api, message, needsLogin, when } from "./api";

const SEVERITIES: [AlertSeverity, string][] = [
  [AlertSeverity.INFORMATIONAL, "Informational"],
  [AlertSeverity.WARNING, "Warning"],
  [AlertSeverity.ERROR, "Error"],
];
const severityName = (s: AlertSeverity) => SEVERITIES.find(([v]) => v === s)?.[1] ?? "";

const TIMES = ["Today", "This week", "This month", "Last Three Months", "Last 6 Months", "This Year", "Everything"];

/** The start of the chosen period in local time; undefined for Everything. */
function openedAfter(time: string): Date | undefined {
  const now = new Date();
  const day = new Date(now.getFullYear(), now.getMonth(), now.getDate());
  switch (time) {
    case "Today":
      return day;
    case "This week": // Weeks start on Monday.
      return new Date(day.getFullYear(), day.getMonth(), day.getDate() - ((day.getDay() + 6) % 7));
    case "This month":
      return new Date(now.getFullYear(), now.getMonth(), 1);
    case "Last Three Months":
      return new Date(now.getFullYear(), now.getMonth() - 3, now.getDate(), now.getHours(), now.getMinutes());
    case "Last 6 Months":
      return new Date(now.getFullYear(), now.getMonth() - 6, now.getDate(), now.getHours(), now.getMinutes());
    case "This Year":
      return new Date(now.getFullYear(), 0, 1);
    default:
      return undefined;
  }
}

/** The header bell: the unhandled Alert count and a menu that leads to the Alerts Overview. */
export function AlertBell({ count }: { count: number }) {
  const [open, setOpen] = useState(false);
  const box = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    const onClick = (e: MouseEvent) => {
      if (!box.current?.contains(e.target as Node)) setOpen(false);
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setOpen(false);
    };
    addEventListener("click", onClick);
    addEventListener("keydown", onKey);
    return () => {
      removeEventListener("click", onClick);
      removeEventListener("keydown", onKey);
    };
  }, [open]);

  return (
    <div class="bell" ref={box}>
      <button
        aria-label={count > 0 ? `Alerts, ${count} unhandled` : "Alerts"}
        aria-expanded={open}
        onClick={() => setOpen(!open)}
      >
        🔔{count > 0 && <span class="bell-count">{count}</span>}
      </button>
      {open && (
        <ul class="bell-menu">
          {count === 0 && <li class="muted">No new alerts</li>}
          <li>
            <a
              href="#/alerts"
              onClick={() => {
                setOpen(false);
              }}
            >
              View All Alerts ({count})
            </a>
          </li>
        </ul>
      )}
    </div>
  );
}

export function AlertsOverview({ onSessionEnded, onChanged }: { onSessionEnded: () => void; onChanged: () => void }) {
  const [environments, setEnvironments] = useState<Environment[]>([]);
  const [environmentId, setEnvironmentId] = useState("");
  const [severity, setSeverity] = useState(AlertSeverity.UNSPECIFIED);
  const [time, setTime] = useState("This month");
  const [includeSnoozed, setIncludeSnoozed] = useState(false);
  const [includeResolved, setIncludeResolved] = useState(false);
  const [alerts, setAlerts] = useState<Alert[] | undefined>(undefined);
  // The last copy of the chosen Alert, so its panel survives leaving the filtered list.
  const [selected, setSelected] = useState<Alert | undefined>(undefined);
  const [note, setNote] = useState("");
  const [error, setError] = useState("");

  const failed = (e: unknown) => {
    if (needsLogin(e)) onSessionEnded();
    else setError(message(e));
  };

  useEffect(() => {
    all(async (pageToken) => {
      const r = await api.listEnvironments({ pageToken });
      return { items: r.environments, next: r.nextPageToken };
    })
      .then(setEnvironments)
      .catch(failed);
  }, []);

  const load = () => {
    const after = openedAfter(time);
    all(async (pageToken) => {
      const r = await api.listAlerts({
        environmentId: environmentId || undefined,
        severity: severity === AlertSeverity.UNSPECIFIED ? undefined : severity,
        openedAfter: after && timestampFromDate(after),
        includeSnoozed,
        includeResolved,
        pageToken,
      });
      return { items: r.alerts, next: r.nextPageToken };
    })
      .then((list) => {
        setAlerts(list);
        setError("");
      })
      .catch(failed);
  };
  useEffect(load, [environmentId, severity, time, includeSnoozed, includeResolved]);

  const act = (call: Promise<Alert>) => {
    call
      .then((a) => {
        setSelected(a);
        setNote("");
        load();
        onChanged();
      })
      .catch(failed);
  };

  const status = (a: Alert) =>
    a.resolvedAt
      ? `Resolved at ${when(a.resolvedAt)}`
      : a.snoozedUntil && a.snoozedUntil.seconds * 1000n > BigInt(Date.now())
        ? `Snoozed until ${when(a.snoozedUntil)}`
        : "Open";

  return (
    <main>
      <div class="toolbar">
        <h2>Alerts Overview</h2>
        <label>
          Environment{" "}
          <select value={environmentId} onChange={(e) => setEnvironmentId(e.currentTarget.value)}>
            <option value="">All</option>
            {environments.map((e) => (
              <option key={e.id} value={e.id}>
                {e.name}
              </option>
            ))}
          </select>
        </label>
        <label>
          Severity{" "}
          <select value={severity} onChange={(e) => setSeverity(Number(e.currentTarget.value))}>
            <option value={AlertSeverity.UNSPECIFIED}>All</option>
            {SEVERITIES.map(([v, name]) => (
              <option key={v} value={v}>
                {name}
              </option>
            ))}
          </select>
        </label>
        <label>
          Time{" "}
          <select value={time} onChange={(e) => setTime(e.currentTarget.value)}>
            {TIMES.map((t) => (
              <option key={t}>{t}</option>
            ))}
          </select>
        </label>
        <label>
          <input type="checkbox" checked={includeSnoozed} onChange={(e) => setIncludeSnoozed(e.currentTarget.checked)} /> Include snoozed
        </label>
        <label>
          <input type="checkbox" checked={includeResolved} onChange={(e) => setIncludeResolved(e.currentTarget.checked)} /> Include resolved
        </label>
      </div>
      {error && <p class="error" role="alert">{error}</p>}
      {alerts === undefined ? (
        <p class="muted">Loading…</p>
      ) : alerts.length === 0 ? (
        <p class="muted">No alerts match these filters.</p>
      ) : (
        <div class="scroll">
        <table>
          <thead>
            <tr>
              <th>Opened</th>
              <th>Environment</th>
              <th>Agent version</th>
              <th>Type</th>
              <th>Severity</th>
              <th>Message</th>
              <th>Last action</th>
            </tr>
          </thead>
          <tbody>
            {alerts.map((a) => (
              <tr key={a.id} class={a.id === selected?.id ? "selected" : ""}>
                <td>
                  <button onClick={() => setSelected(a)}>
                    {when(a.openedAt)}
                  </button>
                </td>
                <td>{a.environmentName}</td>
                <td>{a.agentVersion}</td>
                <td>Offline</td>
                <td>{severityName(a.severity)}</td>
                <td>{a.message}</td>
                <td>{a.lastAction ?? "None"}</td>
              </tr>
            ))}
          </tbody>
        </table>
        </div>
      )}
      {selected && (
        <section class="alert-detail">
          <h3>{selected.hostname}</h3>
          <dl>
            <dt>Opened</dt>
            <dd>{when(selected.openedAt)}</dd>
            <dt>Environment</dt>
            <dd>{selected.environmentName}</dd>
            <dt>Agent version</dt>
            <dd>{selected.agentVersion}</dd>
            <dt>Type</dt>
            <dd>Offline</dd>
            <dt>Severity</dt>
            <dd>{severityName(selected.severity)}</dd>
            <dt>Message</dt>
            <dd>{selected.message}</dd>
            <dt>Status</dt>
            <dd>{status(selected)}</dd>
            <dt>Last action</dt>
            <dd>{selected.lastAction ?? "None"}</dd>
          </dl>
          <div class="actions">
            <button disabled={!!selected.resolvedAt} onClick={() => act(api.resolveAlert({ id: selected.id }))}>
              Resolve
            </button>
            <button disabled={!!selected.resolvedAt} onClick={() => act(api.snoozeAlert({ id: selected.id }))}>
              Snooze 12 hours
            </button>
          </div>
          <form
            class="actions"
            onSubmit={(e) => {
              e.preventDefault();
              act(api.addAlertAction({ id: selected.id, text: note }));
            }}
          >
            <label>
              What did you do?{" "}
              <input aria-label="Action note" maxLength={200} value={note} onInput={(e) => setNote(e.currentTarget.value)} required />
            </label>
            <button type="submit">Add action</button>
          </form>
        </section>
      )}
    </main>
  );
}
