import { useEffect, useState } from "preact/hooks";
import { listDevices, onStateChanged, removeDevice, type DeviceInfo } from "./api";
import { deviceLabel, lastSyncedText, removeQuestion } from "./devices";

/**
 * Settings → Account: the user's devices, with when each last synced, and a
 * way to remove any other one. `removing` is the id of a device a "Not you?
 * Remove it" notice led here about, which opens that device's question.
 */
export function Devices({ removing }: { removing: string | null }) {
  const [devices, setDevices] = useState<DeviceInfo[] | null>(null);
  const [asking, setAsking] = useState<number | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [now, setNow] = useState(() => Math.floor(Date.now() / 1000));

  const refresh = () => {
    setNow(Math.floor(Date.now() / 1000));
    listDevices()
      .then((d) => {
        setDevices(d);
        setError("");
      })
      .catch((e) => setError(String(e)));
  };

  useEffect(() => {
    refresh();
    const unlisten = onStateChanged(refresh);
    const tick = setInterval(refresh, 30_000);
    return () => {
      clearInterval(tick);
      unlisten.then((f) => f());
    };
  }, []);

  useEffect(() => {
    if (removing !== null) setAsking(Number(removing));
  }, [removing]);

  const remove = async (d: DeviceInfo) => {
    setBusy(true);
    try {
      await removeDevice(d.id);
      setAsking(null);
      refresh();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <section aria-labelledby="devices">
      <h3 id="devices">Devices</h3>
      {error && <p class="error" role="alert">{error}</p>}
      {devices === null && !error && <p class="muted">Loading…</p>}
      <ul class="devices">
        {(devices ?? []).map((d) => (
          <li key={d.id}>
            <span class="title">{deviceLabel(d)}</span>
            <span class="muted">{d.portable ? "portable" : "stationary"}</span>
            <span class="when">Last synced: {lastSyncedText(d, now)}</span>
            {!d.this_device && asking !== d.id && (
              <button type="button" onClick={() => setAsking(d.id)}>Remove</button>
            )}
            {!d.this_device && asking === d.id && (
              <p class="notice" role="alertdialog" aria-label={`Remove ${deviceLabel(d)}`}>
                {removeQuestion(d)}{" "}
                <button type="button" disabled={busy} onClick={() => remove(d)}>
                  Remove {deviceLabel(d)}
                </button>
                <button type="button" disabled={busy} onClick={() => setAsking(null)}>
                  Cancel
                </button>
              </p>
            )}
          </li>
        ))}
      </ul>
    </section>
  );
}
