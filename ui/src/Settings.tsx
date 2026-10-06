import { useEffect, useRef, useState } from "preact/hooks";
import {
  appVersion,
  autostartEnabled,
  clearHomeLocation,
  endQuietDevice,
  homeLocation,
  lists as loadLists,
  onStateChanged,
  quietDevice,
  priorities,
  quietHours as loadQuietHours,
  setQuietHours,
  setAutostart,
  setDeviceName,
  setDevicePortable,
  setHomeLocation,
  setLoudestAlert,
  thisDevice,
  type AlertStyle,
  type ListInfo,
  type ThisDevice as ThisDeviceInfo,
  type Place,
  type QuietHours,
  type PriorityInfo,
  type Setup,
} from "./api";
import { Account } from "./Account";
import { coordinateText, describePlace, parseCoordinates } from "./home";
import {
  LOUDEST_CHOICES,
  checkDeviceName,
  describeLoudest,
  describeQuiet,
  isCapped,
  portableNote,
  quietNow,
} from "./device";
import { listName } from "./lists";
import { formatInterval, overdueStyles, styleName } from "./priorities";
import {
  DAYS,
  LENGTHS,
  lengthEnd,
  type Length,
  checkQuietHours,
  describeQuietHours,
  newQuietHours,
  scopeOfValue,
  toggleDay,
  valueOfScope,
} from "./quiet";

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
 * What belongs to the user, on all their devices: quiet hours, and the home
 * location (their Home place), where sun events and daylight are worked out.
 * Both sync with the account, so a change here reaches every device.
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
      <QuietHoursEditor />
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

/**
 * Quiet hours: a snooze-all that recurs, such as 22:00 to 07:00 on weeknights,
 * for all reminders or one list, leaving out Maximum unless included. They
 * are your own setting, so they hold on all your devices; the times are read
 * in the time zone each device is in.
 */
function QuietHoursEditor() {
  const [rules, setRules] = useState<QuietHours[]>([]);
  const [lists, setLists] = useState<ListInfo[]>([]);
  const [loaded, setLoaded] = useState(false);
  const [error, setError] = useState("");
  const [saved, setSaved] = useState(false);
  useEffect(() => {
    loadQuietHours()
      .then(setRules)
      .catch(() => {})
      .finally(() => setLoaded(true));
    loadLists().then(setLists).catch(() => {});
  }, []);
  const change = (i: number, patch: Partial<QuietHours>) => {
    setSaved(false);
    setRules(rules.map((r, n) => (n === i ? { ...r, ...patch } : r)));
  };
  const nameOf = (r: QuietHours) => {
    const l = r.scope.kind === "list" ? lists.find((l) => l.id === (r.scope as { list_id: string }).list_id) : undefined;
    return l ? listName(l) : null;
  };
  const save = (e: Event) => {
    e.preventDefault();
    setError("");
    setSaved(false);
    for (const r of rules) {
      const why = checkQuietHours(r);
      if (why) return setError(why);
    }
    setQuietHours(rules)
      .then(() => setSaved(true))
      .catch((err) => setError(String(err)));
  };
  return (
    <form onSubmit={save} aria-label="Quiet hours">
      <h3>Quiet hours</h3>
      <p class="muted">
        A snooze-all that repeats. While quiet hours last, what is open and anything that fires is
        quiet on all your devices. Reminders still go overdue on schedule, and last-chance alerts
        still come. Maximum priority is left out unless you include it.
      </p>
      {loaded && rules.length === 0 && <p>No quiet hours.</p>}
      {rules.map((r, i) => (
        <fieldset key={i} class="quiet-rule">
          <legend>{describeQuietHours(r, nameOf(r))}</legend>
          <div class="row">
            <label>
              From
              <input type="time" value={r.from} onInput={(e) => change(i, { from: e.currentTarget.value })} />
            </label>
            <label>
              To
              <input type="time" value={r.to} onInput={(e) => change(i, { to: e.currentTarget.value })} />
            </label>
          </div>
          <div class="row" role="group" aria-label="Nights it starts on">
            {DAYS.map(([code, name]) => (
              <label key={code} class="check">
                <input
                  type="checkbox"
                  checked={r.days.includes(code)}
                  onChange={(e) => change(i, { days: toggleDay(r.days, code, e.currentTarget.checked) })}
                />
                {name}
              </label>
            ))}
          </div>
          <label>
            For
            <select
              value={valueOfScope(r.scope)}
              onChange={(e) => change(i, { scope: scopeOfValue(e.currentTarget.value) })}
            >
              <option value="">All reminders</option>
              {lists.map((l) => (
                <option key={l.id} value={l.id}>
                  {listName(l)}
                </option>
              ))}
            </select>
          </label>
          <label class="check">
            <input
              type="checkbox"
              checked={r.include_maximum}
              onChange={(e) => change(i, { include_maximum: e.currentTarget.checked })}
            />
            Include maximum
          </label>
          <button
            type="button"
            onClick={() => {
              setSaved(false);
              setRules(rules.filter((_, n) => n !== i));
            }}
          >
            Remove
          </button>
        </fieldset>
      ))}
      <div class="row">
        <button
          type="button"
          onClick={() => {
            setSaved(false);
            setRules([...rules, newQuietHours()]);
          }}
        >
          Add quiet hours
        </button>
        <button type="submit">Save quiet hours</button>
      </div>
      {saved && <p role="status">Saved.</p>}
      {error && <p role="alert">{error}</p>}
    </form>
  );
}

/**
 * What belongs to this device. Its name and whether it is portable are your
 * own settings, so your other devices see them; the loudest alert and "Quiet
 * this device until…" stay on this device and never sync.
 */
function ThisDevice() {
  const [on, setOn] = useState<boolean | null>(null);
  const [device, setDevice] = useState<ThisDeviceInfo | null>(null);
  const [name, setName] = useState("");
  const [now, setNow] = useState(() => Math.floor(Date.now() / 1000));
  const [error, setError] = useState("");
  const [saved, setSaved] = useState("");
  const [choice, setChoice] = useState(1);
  const [time, setTime] = useState("");
  const [includeMax, setIncludeMax] = useState(false);

  const load = (keepName = true) => {
    setNow(Math.floor(Date.now() / 1000));
    thisDevice()
      .then((d) => {
        setDevice(d);
        if (!keepName) setName(d.name);
      })
      .catch(() => {});
  };
  useEffect(() => {
    autostartEnabled().then(setOn).catch(() => setOn(false));
    load(false);
    // The quiet setting ends by itself: look again as time passes.
    const unlisten = onStateChanged(() => load());
    const timer = setInterval(() => load(), 30_000);
    return () => {
      unlisten.then((f) => f());
      clearInterval(timer);
    };
  }, []);
  const change = (enabled: boolean) => {
    setError("");
    setAutostart(enabled)
      .then(() => setOn(enabled))
      .catch((e) => setError(String(e)));
  };
  const run = (what: string, f: Promise<void>) => {
    setError("");
    setSaved("");
    f.then(() => {
      setSaved(what);
      load();
    }).catch((e) => setError(String(e)));
  };
  const saveName = (e: Event) => {
    e.preventDefault();
    const why = checkDeviceName(name);
    if (why) return setError(why);
    run("Saved.", setDeviceName(name));
  };

  const OWN = LENGTHS.length;
  const length: Length = choice === OWN ? { kind: "time", time } : LENGTHS[choice][1];
  const until = lengthEnd(length, now);
  const quiet = device ? quietNow(device, now) : null;

  return (
    <section aria-labelledby="device">
      <h2 id="device">This device</h2>
      {device && (
        <>
          <form onSubmit={saveName} aria-label="Device name">
            <label>
              Name
              <input value={name} onInput={(e) => setName(e.currentTarget.value)} />
            </label>
            <button type="submit" disabled={name.trim() === device.name}>Save name</button>
            <p class="muted">Your other devices show this name. It is one of your own settings.</p>
          </form>
          <fieldset>
            <legend>Does this device go where you go?</legend>
            <label class="radio">
              <input
                type="radio"
                name="device-portable"
                checked={device.portable}
                onChange={() => run("Saved.", setDevicePortable(true))}
              />
              Portable, like a laptop
            </label>
            <label class="radio">
              <input
                type="radio"
                name="device-portable"
                checked={!device.portable}
                onChange={() => run("Saved.", setDevicePortable(false))}
              />
              Stationary, like a desktop
            </label>
            <p class="muted">
              {portableNote(device.portable)} It starts as stationary unless this computer has a
              battery, and it is one of your own settings.
            </p>
          </fieldset>
          <fieldset>
            <legend>Loudest alert</legend>
            <label>
              The loudest style this device uses
              <select
                value={device.loudest.style}
                onChange={(e) =>
                  run("Saved.", setLoudestAlert(e.currentTarget.value as AlertStyle, device.loudest.caps_maximum))
                }
              >
                {LOUDEST_CHOICES.map(([style, label]) => (
                  <option key={style} value={style}>
                    {label}
                  </option>
                ))}
              </select>
            </label>
            <label class="check">
              <input
                type="checkbox"
                checked={device.loudest.caps_maximum}
                disabled={!isCapped(device.loudest)}
                onChange={(e) => run("Saved.", setLoudestAlert(device.loudest.style, e.currentTarget.checked))}
              />
              Cap Maximum too
            </label>
            <p class="muted" role="status">
              {describeLoudest(device.loudest)}. Louder alerts are downgraded on this device only,
              such as no alarms on the desktop; your other devices alert as they are set. This
              stays on this device.
            </p>
          </fieldset>
          <fieldset>
            <legend>Quiet this device until…</legend>
            {quiet && (
              <p class="chip" role="status">
                {describeQuiet(quiet, now)} ·{" "}
                <button type="button" onClick={() => run("", endQuietDevice())}>
                  End now
                </button>
              </p>
            )}
            {LENGTHS.map(([label], i) => (
              <label class="radio" key={label}>
                <input type="radio" name="quiet-device-length" checked={choice === i} onChange={() => setChoice(i)} />
                {label}
              </label>
            ))}
            <label class="radio">
              <input type="radio" name="quiet-device-length" checked={choice === OWN} onChange={() => setChoice(OWN)} />
              A time
              <input
                type="time"
                aria-label="Quiet this device until a time"
                value={time}
                onFocus={() => setChoice(OWN)}
                onInput={(e) => {
                  setTime(e.currentTarget.value);
                  setChoice(OWN);
                }}
              />
            </label>
            <label class="check">
              <input type="checkbox" checked={includeMax} onChange={(e) => setIncludeMax(e.currentTarget.checked)} />
              Include maximum
            </label>
            <button
              type="button"
              disabled={until === null}
              onClick={() => until !== null && run("", quietDevice(until, includeMax))}
            >
              Quiet this device
            </button>
            <p class="muted">
              Everything on this device is silent until then; Maximum priority still alerts unless
              you include it. It isn't a snooze: your other devices still alert, and reminders go
              overdue on schedule. It stays on this device.
            </p>
          </fieldset>
        </>
      )}
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
      {saved && <p role="status">{saved}</p>}
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
