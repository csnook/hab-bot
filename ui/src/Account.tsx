import { useEffect, useState } from "preact/hooks";
import { keyStoreName, type Setup } from "./api";
import { ServerSees } from "./ServerSees";

/** Settings → Account. */
export function Account({ setup }: { setup: Setup }) {
  const [keys, setKeys] = useState("");
  useEffect(() => {
    keyStoreName().then(setKeys).catch(() => setKeys(""));
  }, []);

  if (setup.mode === "standalone") {
    return (
      <section aria-labelledby="account">
        <h2 id="account">Account</h2>
        <p>This device is on its own: there is no account and no server.</p>
      </section>
    );
  }

  return (
    <section aria-labelledby="account">
      <h2 id="account">Account</h2>
      <dl>
        <dt>Display name</dt>
        <dd>{setup.display_name}</dd>
        <dt>Username</dt>
        <dd>{setup.username}</dd>
        <dt>Role</dt>
        <dd>{setup.admin ? "Server admin" : "Member"}</dd>
        <dt>Server</dt>
        <dd>
          {setup.server_name} ({setup.server_address})
        </dd>
        <dt>Pinned certificate (SHA-256)</dt>
        <dd class="fingerprint">{setup.server_fingerprint}</dd>
        <dt>This device</dt>
        <dd>
          {setup.device_name}, {setup.portable ? "portable" : "stationary"}
        </dd>
        {keys && (
          <>
            <dt>Keys are kept in</dt>
            <dd>{keys}</dd>
          </>
        )}
      </dl>
      <ServerSees />
    </section>
  );
}
