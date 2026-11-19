import { render } from "preact";
import { useEffect, useState } from "preact/hooks";
import type { Endpoint, Environment } from "./gen/rustyrmm/operator/v1/operator_pb";
import { all, api, message, needsLogin, when } from "./api";
import { Environments, Operators } from "./admin";
import { EndpointDetail } from "./detail";
import "./style.css";

function App() {
  // undefined = still checking whether the cookie session is valid.
  const [loggedIn, setLoggedIn] = useState<boolean | undefined>(undefined);
  const [hash, setHash] = useState(location.hash);

  useEffect(() => {
    const onHash = () => {
      setHash(location.hash);
    };
    addEventListener("hashchange", onHash);
    return () => {
      removeEventListener("hashchange", onHash);
    };
  }, []);

  useEffect(() => {
    api
      .listEnvironments({ pageSize: 1 })
      .then(() => setLoggedIn(true))
      .catch((e) => setLoggedIn(needsLogin(e) ? false : true));
  }, []);

  if (loggedIn === undefined) return <p class="muted">Loading…</p>;
  if (!loggedIn) return <Login onDone={() => setLoggedIn(true)} />;
  const detailId = /^#\/endpoints\/([^/]+)$/.exec(hash)?.[1];
  const endSession = () => {
    setLoggedIn(false);
  };
  const nav = (href: string, label: string, active: boolean) => (
    <a href={href} aria-current={active ? "page" : undefined}>
      {label}
    </a>
  );
  return (
    <>
      <header>
        <nav>
          <strong>RustyRMM</strong>
          {nav("#", "Endpoints", hash === "" || hash === "#" || detailId !== undefined)}
          {nav("#/environments", "Environments", hash === "#/environments")}
          {nav("#/operators", "Operators", hash === "#/operators")}
        </nav>
        <button
          onClick={() => {
            void api.logout({}).catch(() => {}).finally(() => setLoggedIn(false));
          }}
        >
          Log out
        </button>
      </header>
      {detailId ? (
        <EndpointDetail id={detailId} onSessionEnded={endSession} />
      ) : hash === "#/environments" ? (
        <Environments onSessionEnded={endSession} />
      ) : hash === "#/operators" ? (
        <Operators onSessionEnded={endSession} />
      ) : (
        <Endpoints onSessionEnded={endSession} />
      )}
    </>
  );
}

function Login({ onDone }: { onDone: () => void }) {
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);

  async function submit(e: Event) {
    e.preventDefault();
    setBusy(true);
    setError("");
    try {
      // The response sets the HttpOnly session cookie; the token in the body is for the CLI.
      await api.login({ username, password });
      onDone();
    } catch (err) {
      setError(message(err));
    } finally {
      setBusy(false);
    }
  }

  return (
    <form class="login" onSubmit={(e) => { void submit(e); }}>
      <h1>RustyRMM</h1>
      <label>
        Username
        <input autocomplete="username" value={username} onInput={(e) => setUsername(e.currentTarget.value)} required />
      </label>
      <label>
        Password
        <input type="password" autocomplete="current-password" value={password} onInput={(e) => setPassword(e.currentTarget.value)} required />
      </label>
      {error && <p class="error" role="alert">{error}</p>}
      <button type="submit" disabled={busy}>{busy ? "Logging in…" : "Log in"}</button>
    </form>
  );
}

function Endpoints({ onSessionEnded }: { onSessionEnded: () => void }) {
  const [environments, setEnvironments] = useState<Environment[]>([]);
  const [environmentId, setEnvironmentId] = useState("");
  const [endpoints, setEndpoints] = useState<Endpoint[] | undefined>(undefined);
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

  useEffect(() => {
    const load = () => {
      void all(async (pageToken) => {
        const r = await api.listEndpoints({ environmentId: environmentId || undefined, pageToken });
        return { items: r.endpoints, next: r.nextPageToken };
      })
        .then((list) => {
          setEndpoints(list);
          setError("");
        })
        .catch(failed);
    };
    load();
    const timer = setInterval(load, 30_000); // Online status changes; refresh like the charts will.
    return () => {
      clearInterval(timer);
    };
  }, [environmentId]);

  const envName = (id: string) => environments.find((e) => e.id === id)?.name ?? "";

  return (
    <main>
      <div class="toolbar">
        <h2>Endpoints</h2>
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
      </div>
      {error && <p class="error" role="alert">{error}</p>}
      {endpoints === undefined ? (
        <p class="muted">Loading…</p>
      ) : endpoints.length === 0 ? (
        <p class="muted">No Endpoints yet. Create a Pairing Token and install the Agent on a machine.</p>
      ) : (
        <table>
          <thead>
            <tr>
              <th>Status</th>
              <th>Hostname</th>
              <th>Environment</th>
              <th>OS</th>
              <th>Last seen</th>
            </tr>
          </thead>
          <tbody>
            {endpoints.map((e) => (
              <tr key={e.id}>
                <td>
                  <span class={e.revoked ? "badge revoked" : e.online ? "badge online" : "badge offline"}>
                    {e.revoked ? "Revoked" : e.online ? "Online" : "Offline"}
                  </span>
                </td>
                <td>
                  <a href={`#/endpoints/${e.id}`}>{e.hostname || <span class="muted">(not reported yet)</span>}</a>
                </td>
                <td>{envName(e.environmentId)}</td>
                <td>{e.inventory?.os?.name ?? ""}</td>
                <td>{e.online ? "now" : when(e.lastSeenAt)}</td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </main>
  );
}

const root = document.getElementById("app");
if (root) render(<App />, root);
