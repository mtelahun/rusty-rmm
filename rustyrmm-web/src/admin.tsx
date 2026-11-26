import { useEffect, useState } from "preact/hooks";
import { timestampDate } from "@bufbuild/protobuf/wkt";
import type { CreatePairingTokenResponse, Environment, OperatorInfo, PairingToken } from "./gen/rustyrmm/operator/v1/operator_pb";
import { all, api, message, needsLogin, THRESHOLD_ERROR, thresholdMinutes, when } from "./api";

// Same one-liners as `rustyrmm pairing-token create` (rustyrmm-cli/src/main.rs).
const RELEASES = "https://github.com/mtelahun/rusty-rmm/releases/latest/download";

const EXPIRIES: [number, string][] = [
  [0, "Server default (7 days)"],
  [3600, "1 hour"],
  [86400, "1 day"],
  [30 * 86400, "30 days"],
];

function failHandler(onSessionEnded: () => void, setError: (m: string) => void) {
  return (e: unknown) => {
    if (needsLogin(e)) onSessionEnded();
    else setError(message(e));
  };
}

export function Environments({ isAdmin, onSessionEnded }: { isAdmin: boolean; onSessionEnded: () => void }) {
  const [environments, setEnvironments] = useState<Environment[] | undefined>(undefined);
  const [selected, setSelected] = useState("");
  const [name, setName] = useState("");
  const [owner, setOwner] = useState("");
  const [error, setError] = useState("");
  const failed = failHandler(onSessionEnded, setError);

  const load = () => {
    all(async (pageToken) => {
      const r = await api.listEnvironments({ pageToken });
      return { items: r.environments, next: r.nextPageToken };
    })
      .then((list) => {
        setEnvironments(list);
        setError("");
      })
      .catch(failed);
  };
  useEffect(load, []);

  const create = (e: Event) => {
    e.preventDefault();
    api
      .createEnvironment({ name, owner })
      .then((env) => {
        setName("");
        setOwner("");
        setSelected(env.id);
        load();
      })
      .catch(failed);
  };

  const current = environments?.find((e) => e.id === selected);

  return (
    <main>
      <div class="toolbar">
        <h2>Environments</h2>
        {isAdmin && (
          <form class="inline" onSubmit={create}>
            <input
              aria-label="New Environment name"
              placeholder="New Environment name"
              value={name}
              onInput={(e) => {
                setName(e.currentTarget.value);
              }}
              required
            />
            <input
              aria-label="Owner (optional)"
              placeholder="Owner (optional)"
              maxLength={200}
              value={owner}
              onInput={(e) => {
                setOwner(e.currentTarget.value);
              }}
            />
            <button type="submit">Create</button>
          </form>
        )}
      </div>
      {error && <p class="error" role="alert">{error}</p>}
      {environments === undefined ? (
        <p class="muted">Loading…</p>
      ) : environments.length === 0 ? (
        <p class="muted">No Environments yet. Create one to start pairing Endpoints.</p>
      ) : (
        <div class="scroll">
          <table>
            <thead>
              <tr>
                <th>Name</th>
                <th>Owner</th>
                <th>Created</th>
              </tr>
            </thead>
            <tbody>
              {environments.map((env) => (
                <tr key={env.id} class={env.id === selected ? "selected" : ""}>
                  <td>
                    <a
                      href="#/environments"
                      onClick={(e) => {
                        e.preventDefault();
                        setSelected(env.id);
                      }}
                    >
                      {env.name}
                    </a>
                  </td>
                  <td>{env.owner}</td>
                  <td>{when(env.createdAt)}</td>
                </tr>
              ))}
            </tbody>
        </table>
        </div>
      )}
      {current && <OwnerForm key={current.id} environment={current} saved={load} failed={failed} />}
      {current && isAdmin && <AlertSettings key={current.id} environment={current} saved={load} failed={failed} />}
      {current && <PairingTokens key={current.id} environment={current} failed={failed} />}
    </main>
  );
}

function OwnerForm({ environment, saved, failed }: { environment: Environment; saved: () => void; failed: (e: unknown) => void }) {
  const [owner, setOwner] = useState(environment.owner);
  const save = (e: Event) => {
    e.preventDefault();
    api.updateEnvironment({ id: environment.id, owner }).then(saved).catch(failed);
  };
  return (
    <section>
      <div class="toolbar">
        <h3>Owner for {environment.name}</h3>
        <form class="inline" onSubmit={save}>
          <label>
            Owner{" "}
            <input
              maxLength={200}
              value={owner}
              onInput={(e) => {
                setOwner(e.currentTarget.value);
              }}
            />
          </label>
          <button type="submit">Save</button>
        </form>
      </div>
    </section>
  );
}

function AlertSettings({ environment, saved, failed }: { environment: Environment; saved: () => void; failed: (e: unknown) => void }) {
  const [threshold, setThreshold] = useState(environment.offlineThresholdMinutes.toString());
  const [recipients, setRecipients] = useState(environment.alertRecipients);
  const [operators, setOperators] = useState<OperatorInfo[]>([]);
  const [invalid, setInvalid] = useState(false);

  useEffect(() => {
    api
      .listOperators({})
      .then((r) => {
        setOperators(r.operators);
      })
      .catch(failed);
  }, []);

  const offered = operators.filter((o) => (o.isAdmin || o.environmentIds.includes(environment.id)) && !recipients.includes(o.username));
  const save = (e: Event) => {
    e.preventDefault();
    const offlineThresholdMinutes = thresholdMinutes(threshold);
    setInvalid(offlineThresholdMinutes === undefined);
    if (offlineThresholdMinutes === undefined) return;
    api
      .setEnvironmentAlertSettings({ environmentId: environment.id, offlineThresholdMinutes, alertRecipients: recipients })
      .then(saved)
      .catch(failed);
  };
  return (
    <section>
      <h3>Alerts for {environment.name}</h3>
      <form class="inline" onSubmit={save}>
        <label>
          Offline threshold (minutes){" "}
          <input
            inputMode="numeric"
            required
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
      <h4>Recipients</h4>
      {recipients.length === 0 && <p class="muted">No recipients.</p>}
      <ul>
        {recipients.map((r) => (
          <li key={r}>
            {r}{" "}
            <button
              type="button"
              aria-label={`Remove ${r}`}
              onClick={() => {
                setRecipients(recipients.filter((x) => x !== r));
              }}
            >
              Remove
            </button>
          </li>
        ))}
      </ul>
      {offered.length > 0 && (
        <select
          aria-label="Add recipient"
          value=""
          onChange={(e) => {
            setRecipients([...recipients, e.currentTarget.value]);
          }}
        >
          <option value="">Add recipient…</option>
          {offered.map((o) => (
            <option key={o.username} value={o.username}>
              {o.username}
            </option>
          ))}
        </select>
      )}
    </section>
  );
}

function tokenState(t: PairingToken): string {
  if (t.revoked) return "Revoked";
  if (t.expiresAt && timestampDate(t.expiresAt).getTime() < Date.now()) return "Expired";
  return "Active";
}

function PairingTokens({ environment, failed }: { environment: Environment; failed: (e: unknown) => void }) {
  const [tokens, setTokens] = useState<PairingToken[] | undefined>(undefined);
  const [expiry, setExpiry] = useState(0);
  const [created, setCreated] = useState<CreatePairingTokenResponse | undefined>(undefined);
  const [busy, setBusy] = useState(false);

  const load = () => {
    all(async (pageToken) => {
      const r = await api.listPairingTokens({ environmentId: environment.id, pageToken });
      return { items: r.pairingTokens, next: r.nextPageToken };
    })
      .then((list) => {
        setTokens(list.reverse()); // Newest first.
      })
      .catch(failed);
  };
  useEffect(load, [environment.id]);

  const create = () => {
    setBusy(true);
    api
      .createPairingToken({ environmentId: environment.id, ttl: expiry ? { seconds: BigInt(expiry) } : undefined })
      .then((r) => {
        setCreated(r);
        load();
      })
      .catch(failed)
      .finally(() => {
        setBusy(false);
      });
  };

  const revoke = (id: string) => {
    if (!confirm("Revoke this Pairing Token? Agents can no longer pair with it.")) return;
    api.revokePairingToken({ id }).then(load).catch(failed);
  };

  return (
    <section>
      <div class="toolbar">
        <h3>Pairing Tokens for {environment.name}</h3>
        <div class="actions">
          <label>
            Expires{" "}
            <select
              value={expiry}
              onChange={(e) => {
                setExpiry(Number(e.currentTarget.value));
              }}
            >
              {EXPIRIES.map(([secs, label]) => (
                <option key={secs} value={secs}>
                  {label}
                </option>
              ))}
            </select>
          </label>
          <button onClick={create} disabled={busy}>
            Create Pairing Token
          </button>
        </div>
      </div>
      {created && (
        <InstallCommands
          created={created}
          onDismiss={() => {
            setCreated(undefined);
          }}
        />
      )}
      {tokens === undefined ? (
        <p class="muted">Loading…</p>
      ) : tokens.length === 0 ? (
        <p class="muted">No Pairing Tokens for this Environment.</p>
      ) : (
        <table>
          <thead>
            <tr>
              <th>Id</th>
              <th>Created</th>
              <th>Expires</th>
              <th>Status</th>
              <th />
            </tr>
          </thead>
          <tbody>
            {tokens.map((t) => {
              const state = tokenState(t);
              return (
                <tr key={t.id}>
                  <td class="cmdline">{t.id}</td>
                  <td>{when(t.createdAt)}</td>
                  <td>{when(t.expiresAt)}</td>
                  <td>
                    <span class={state === "Active" ? "badge online" : state === "Revoked" ? "badge revoked" : "badge offline"}>{state}</span>
                  </td>
                  <td>
                    {state === "Active" && (
                      <button
                        onClick={() => {
                          revoke(t.id);
                        }}
                      >
                        Revoke
                      </button>
                    )}
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
      )}
    </section>
  );
}

function InstallCommands({ created, onDismiss }: { created: CreatePairingTokenResponse; onDismiss: () => void }) {
  const pair = `pair ${created.secret} --server ${created.agentAddress} --ca-fingerprint ${created.caFingerprintSha256} --install-service`;
  const commands: [string, string][] = [
    ["Linux / macOS", `curl --proto '=https' --tlsv1.2 -LsSf ${RELEASES}/rustyrmm-agent-installer.sh | sh && sudo ~/.cargo/bin/rustyrmm-agent ${pair}`],
    [
      "Windows (admin PowerShell)",
      `powershell -ExecutionPolicy Bypass -c "[Net.ServicePointManager]::SecurityProtocol = 'Tls12'; irm ${RELEASES}/rustyrmm-agent-installer.ps1 | iex"; & "$env:USERPROFILE\\.cargo\\bin\\rustyrmm-agent.exe" ${pair}`,
    ],
  ];
  return (
    <div class="notice" role="status">
      <p>
        <strong>Copy an install command now: the secret is shown only once.</strong>
      </p>
      {commands.map(([label, cmd]) => (
        <div key={label}>
          <h4>{label}</h4>
          <pre>{cmd}</pre>
          <button
            onClick={() => {
              void navigator.clipboard.writeText(cmd);
            }}
          >
            Copy
          </button>
        </div>
      ))}
      <p>
        <button onClick={onDismiss}>Done</button>
      </p>
    </div>
  );
}

/** An email address with an Edit button; saving calls onSaved so the parent can reload. */
function EmailEditor({ username, email, onSaved, onSessionEnded }: { username: string; email: string; onSaved: () => void; onSessionEnded: () => void }) {
  const [editing, setEditing] = useState(false);
  const [value, setValue] = useState(email);
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const failed = failHandler(onSessionEnded, setError);

  const save = (e: Event) => {
    e.preventDefault();
    setBusy(true);
    setError("");
    api
      .setOperatorEmail({ username, email: value })
      .then(() => {
        setEditing(false);
        onSaved();
      })
      .catch(failed)
      .finally(() => {
        setBusy(false);
      });
  };

  if (!editing) {
    return (
      <>
        {email}{" "}
        <button
          aria-label={`Edit email address of ${username}`}
          onClick={() => {
            setValue(email);
            setEditing(true);
          }}
        >
          Edit
        </button>
      </>
    );
  }
  return (
    <form class="inline" onSubmit={save}>
      <input
        type="email"
        aria-label={`Email address of ${username}`}
        value={value}
        onInput={(e) => {
          setValue(e.currentTarget.value);
        }}
        required
      />{" "}
      <button type="submit" disabled={busy}>
        {busy ? "Saving…" : "Save"}
      </button>{" "}
      <button
        type="button"
        onClick={() => {
          setEditing(false);
          setError("");
        }}
      >
        Cancel
      </button>
      {error && (
        <p class="error" role="alert">
          {error}
        </p>
      )}
    </form>
  );
}

export function Operators({ isAdmin, onSessionEnded }: { isAdmin: boolean; onSessionEnded: () => void }) {
  const [username, setUsername] = useState("");
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [shown, setShown] = useState(false); // Instead of a confirm field: the Operator can check for typos.
  const [error, setError] = useState("");
  const [added, setAdded] = useState("");
  const [busy, setBusy] = useState(false);
  const [operators, setOperators] = useState<OperatorInfo[] | undefined>(undefined);
  const [me, setMe] = useState<{ username: string; email: string } | undefined>(undefined);
  const failed = failHandler(onSessionEnded, setError);

  const load = () => {
    api
      .whoAmI({})
      .then((r) => {
        setMe({ username: r.username, email: r.email ?? "" });
      })
      .catch(failed);
    if (isAdmin) {
      api
        .listOperators({})
        .then((r) => {
          setOperators(r.operators);
        })
        .catch(failed);
    }
  };
  useEffect(load, [isAdmin]);

  const submit = (e: Event) => {
    e.preventDefault();
    setAdded("");
    setBusy(true);
    setError("");
    api
      .addOperator({ username, password, email })
      .then(() => {
        setAdded(username);
        setUsername("");
        setEmail("");
        setPassword("");
        setShown(false);
        load();
      })
      .catch(failed)
      .finally(() => {
        setBusy(false);
      });
  };

  return (
    <main>
      <h2>Your email address</h2>
      {me && <EmailEditor key={me.email} username={me.username} email={me.email} onSaved={load} onSessionEnded={onSessionEnded} />}
      {isAdmin && (
        <>
          <h2>Operators</h2>
          {operators && (
            <div class="scroll">
            <table>
              <thead>
                <tr>
                  <th>Username</th>
                  <th>Email</th>
                  <th>Admin</th>
                </tr>
              </thead>
              <tbody>
                {operators.map((o) => (
                  <tr key={o.username}>
                    <td>{o.username}</td>
                    <td>
                      <EmailEditor key={o.email ?? ""} username={o.username} email={o.email ?? ""} onSaved={load} onSessionEnded={onSessionEnded} />
                    </td>
                    <td>{o.isAdmin ? "Yes" : ""}</td>
                  </tr>
                ))}
              </tbody>
            </table>
            </div>
          )}
        </>
      )}
      <h2>Add an Operator</h2>
      {isAdmin ? (
        <form class="stack" onSubmit={submit}>
          <label>
            Username
            <input
              autocomplete="off"
              value={username}
              onInput={(e) => {
                setUsername(e.currentTarget.value);
              }}
              required
            />
          </label>
          <label>
            Email address
            <input
              type="email"
              autocomplete="off"
              value={email}
              onInput={(e) => {
                setEmail(e.currentTarget.value);
              }}
              required
            />
          </label>
          <label>
            Password (at least 12 characters)
            <input
              {...(shown ? { type: "text" as const } : { type: "password" as const })}
              autocomplete="new-password"
              minLength={12}
              value={password}
              onInput={(e) => {
                setPassword(e.currentTarget.value);
              }}
              required
            />
          </label>
          <label class="check">
            <input
              type="checkbox"
              checked={shown}
              onChange={(e) => {
                setShown(e.currentTarget.checked);
              }}
            />
            Show password
          </label>
          {error && <p class="error" role="alert">{error}</p>}
          {added && <p role="status">Added Operator {added}.</p>}
          <button type="submit" disabled={busy}>
            {busy ? "Adding…" : "Add Operator"}
          </button>
        </form>
      ) : (
        <p class="muted">Only Administrators can add Operators.</p>
      )}
    </main>
  );
}
