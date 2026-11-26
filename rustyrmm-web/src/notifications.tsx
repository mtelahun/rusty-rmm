import { useEffect, useState } from "preact/hooks";
import type { Duration } from "@bufbuild/protobuf/wkt";
import { NotificationKind, type Notification } from "./gen/rustyrmm/operator/v1/operator_pb";
import { api, message, needsLogin, when } from "./api";

function duration(d: Duration): string {
  const minutes = Math.floor(Number(d.seconds) / 60);
  return minutes < 60 ? `${minutes} min` : `${Math.floor(minutes / 60)} h ${minutes % 60} min`;
}

export function Notifications({ onSessionEnded, onChanged }: { onSessionEnded: () => void; onChanged: () => void }) {
  const [items, setItems] = useState<Notification[] | undefined>(undefined);
  const [next, setNext] = useState("");
  const [error, setError] = useState("");

  const failed = (e: unknown) => {
    if (needsLogin(e)) onSessionEnded();
    else setError(message(e));
  };

  const load = (pageToken: string) =>
    api
      .listNotifications({ pageToken })
      .then((r) => {
        setItems((old) => (pageToken ? [...(old ?? []), ...r.notifications] : r.notifications));
        setNext(r.nextPageToken);
        setError("");
      })
      .catch(failed);

  useEffect(() => {
    void load("");
  }, []);

  const mark = (call: () => Promise<unknown>) => {
    void call()
      .then(() => {
        onChanged();
        return load("");
      })
      .catch(failed);
  };

  return (
    <main>
      <div class="toolbar">
        <h2>Notifications</h2>
        <button onClick={() => mark(() => api.markAllNotificationsRead({}))}>Mark all read</button>
      </div>
      {error && <p class="error" role="alert">{error}</p>}
      {items === undefined ? (
        <p class="muted">Loading…</p>
      ) : items.length === 0 ? (
        <p class="muted">No Notifications.</p>
      ) : (
        <div class="scroll">
        <table>
          <thead>
            <tr>
              <th>Kind</th>
              <th>Hostname</th>
              <th>Last seen</th>
              <th>Duration</th>
              <th></th>
            </tr>
          </thead>
          <tbody>
            {items.map((n) => (
              <tr key={n.id} class={n.readAt ? undefined : "unread"}>
                <td>{n.kind === NotificationKind.WENT_OFFLINE ? "Went offline" : "Back online"}</td>
                <td>{n.hostname}</td>
                <td>{when(n.lastSeenAt)}</td>
                <td>{n.offlineDuration ? `Offline for ${duration(n.offlineDuration)}` : ""}</td>
                <td>
                  {!n.readAt && (
                    <>
                      <span class="badge">Unread</span>{" "}
                      <button onClick={() => mark(() => api.markNotificationRead({ id: n.id }))}>Mark read</button>
                    </>
                  )}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
        </div>
      )}
      {next && <button onClick={() => void load(next)}>Load more</button>}
    </main>
  );
}
