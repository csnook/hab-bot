import { useEffect, useRef, useState } from "preact/hooks";
import {
  appVersion,
  autostartEnabled,
  clearHomeLocation,
  homeLocation,
  priorities,
  setAutostart,
  setHomeLocation,
  type Place,
  type PriorityInfo,
  type Setup,
} from "./api";
import { Account } from "./Account";
import { coordinateText, describePlace, parseCoordinates } from "./home";
import { formatInterval, overdueStyles, styleName } from "./priorities";

export type Section = "you" | "priorities" | "device" | "account" | "about";
const SECTIONS: Array<[Section, string]> = [
  ["you", "You"],
  ["priorities", "Priorities"],
  ["device", "This device"],
  ["account", "Account"],
  ["about", "About"],
];

/** One dialog, opened from the gear: Settings. */
export function Settings({
  setup,
  removing,
  start,
  onClose,
}: {
  setup: Setup;
  removing: string | null;
  start: Section;
  onClose: () => void;
}) {
  const ref = useRef<HTMLDialogElement>(null);
  const [section, setSection] = useState<Section>(start);

  useEffect(() => {
    const d = ref.current;
    if (d && !d.open) {
      try {
        d.showModal();
      } catch {
        d.setAttribute("open", "");
      }
    }
  }, []);

  return (
    <dialog ref={ref} class="settings" aria-labelledby="settings-title" onClose={onClose}>
      <header>
        <h1 id="settings-title">Settings</h1>
        <button type="button" onClick={onClose} aria-label="Close settings">Close</button>
      </header>
      <nav class="tabs" aria-label="Settings sections">
        {SECTIONS.map(([id, label]) => (
          <button key={id} aria-pressed={section === id} onClick={() => setSection(id)}>
            {label}
          </button>
        ))}
      </nav>
      {section === "you" && <You />}
      {section === "priorities" && <Priorities />}
      {section === "device" && <ThisDevice />}
      {section === "account" && <Account setup={setup} removing={removing} />}
      {section === "about" && <About setup={setup} />}
    </dialog>
  );
}

/**
 * What belongs to the user, on all their devices. In this release that is the
 * home location (their Home place): where sun events and daylight are worked
 * out. It syncs with the account, so a change here reaches every device.
 */
function You() {
  const [home, setHome] = useState<Place | null>(null);
  const [loaded, setLoaded] = useState(false);
  const [lat, setLat] = useState("");
  const [lon, setLon] = useState("");
  const [error, setError] = useState("");
  const [saved, setSaved] = useState(false);
  useEffect(() => {
    homeLocation()
      .then((h) => {
        setHome(h);
        if (h) {
          setLat(coordinateText(h.latitude));
          setLon(coordinateText(h.longitude));
        }
      })
      .catch(() => {})
      .finally(() => setLoaded(true));
  }, []);
  const save = (e: Event) => {
    e.preventDefault();
    setError("");
    setSaved(false);
    const c = parseCoordinates(lat, lon);
    if (!c.ok) return setError(c.error);
    setHomeLocation(c.value.latitude, c.value.longitude)
      .then(() => homeLocation())
      .then((h) => {
        setHome(h);
        setSaved(true);
      })
      .catch((err) => setError(String(err)));
  };
  const clear = () => {
    setError("");
    setSaved(false);
    clearHomeLocation()
      .then(() => {
        setHome(null);
        setLat("");
        setLon("");
      })
      .catch((err) => setError(String(err)));
  };
  return (
    <section aria-labelledby="you">
      <h2 id="you">You</h2>
      <form onSubmit={save} aria-label="Home location">
        <h3>Home location</h3>
        <p class="muted">
          Sun events (sunrise, sunset, civil dawn and dusk) and the daylight and darkness
          conditions use it. It is your Home place, shared by all your devices; it isn't used for
          arriving or leaving yet.
        </p>
        <p>{!loaded ? "" : home ? `Set to ${describePlace(home)}.` : "Not set yet."}</p>
        <div class="row">
          <label>
            Latitude
            <input
              inputMode="decimal"
              placeholder="51.5074"
              value={lat}
              onInput={(e) => setLat(e.currentTarget.value)}
            />
          </label>
          <label>
            Longitude
            <input
              inputMode="decimal"
              placeholder="-0.1278"
              value={lon}
              onInput={(e) => setLon(e.currentTarget.value)}
            />
          </label>
        </div>
        <div class="row">
          <button type="submit">Save home location</button>
          {home && (
            <button type="button" onClick={clear}>
              Clear
            </button>
          )}
        </div>
        {saved && <p role="status">Saved.</p>}
        {error && <p role="alert">{error}</p>}
      </form>
    </section>
  );
}

/** What belongs to this device alone: not synced, and not the account's. */
function ThisDevice() {
  const [on, setOn] = useState<boolean | null>(null);
  const [error, setError] = useState("");
  useEffect(() => {
    autostartEnabled().then(setOn).catch(() => setOn(false));
  }, []);
  const change = (enabled: boolean) => {
    setError("");
    setAutostart(enabled)
      .then(() => setOn(enabled))
      .catch((e) => setError(String(e)));
  };
  return (
    <section aria-labelledby="device">
      <h2 id="device">This device</h2>
      <label>
        <input
          type="checkbox"
          checked={on === true}
          disabled={on === null}
          onChange={(e) => change((e.currentTarget as HTMLInputElement).checked)}
        />{" "}
        Start at login
      </label>
      <p class="muted">
        Starts Reminders in the tray with no window, so reminders fire after you sign in. Closing
        the window keeps it running there.
      </p>
      {error && <p role="alert">{error}</p>}
    </section>
  );
}

/** The built-ins, read-only. */
function Priorities() {
  const [list, setList] = useState<PriorityInfo[]>([]);
  useEffect(() => {
    priorities().then(setList).catch(() => {});
  }, []);
  return (
    <section aria-labelledby="priorities">
      <h2 id="priorities">Priorities</h2>
      <p class="muted">The built-in priorities can't be edited.</p>
      <table>
        <thead>
          <tr>
            <th scope="col">Priority</th>
            <th scope="col">Due style</th>
            <th scope="col">Due interval</th>
            <th scope="col">Once overdue</th>
            <th scope="col">Overdue interval</th>
            <th scope="col">Swipe (as snooze)</th>
            <th scope="col">Server wait</th>
            <th scope="col">Breaks Do Not Disturb</th>
          </tr>
        </thead>
        <tbody>
          {list.map((p) => (
            <tr key={p.priority}>
              <th scope="row">{p.name}</th>
              <td>{styleName(p.settings.due_style)}</td>
              <td>{formatInterval(p.settings.due_interval)}</td>
              <td>{overdueStyles(p)}</td>
              <td>{formatInterval(p.settings.overdue_interval)}</td>
              <td>{p.settings.swipeable ? "Yes" : "No"}</td>
              <td>
                {p.settings.server_wait === null ? "None" : formatInterval(p.settings.server_wait)}
              </td>
              <td>{p.settings.breaks_do_not_disturb ? "Yes" : "No"}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </section>
  );
}

function About({ setup }: { setup: Setup }) {
  const [version, setVersion] = useState("");
  useEffect(() => {
    appVersion().then(setVersion).catch(() => {});
  }, []);
  return (
    <section aria-labelledby="about">
      <h2 id="about">About</h2>
      <dl>
        <dt>Version</dt>
        <dd>{version}</dd>
        {setup.mode === "joined" && (
          <>
            <dt>Server</dt>
            <dd>{setup.server_name} ({setup.server_address})</dd>
            <dt>Certificate (SHA-256)</dt>
            <dd class="fingerprint">{setup.server_fingerprint}</dd>
          </>
        )}
      </dl>
    </section>
  );
}
