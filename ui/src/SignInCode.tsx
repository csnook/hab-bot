import { useEffect, useState } from "preact/hooks";
import { signInCode, type SignInCode as Code } from "./api";

/** Settings → Account: the code another device signs in with. */
export function SignInCodeBox() {
  const [code, setCode] = useState<Code | null>(null);
  const [error, setError] = useState("");
  const [copied, setCopied] = useState(false);

  useEffect(() => {
    signInCode().then(setCode).catch((e) => setError(String(e)));
  }, []);

  const copy = async () => {
    if (!code) return;
    try {
      await navigator.clipboard.writeText(code.link);
      setCopied(true);
    } catch {
      setCopied(false);
    }
  };

  return (
    <section aria-labelledby="signin-code">
      <h3 id="signin-code">Sign-in code</h3>
      <p class="muted">
        To sign in on another computer, choose Sign in there and enter this code, or its link. It
        carries the server's name and certificate fingerprint. You still need your username and
        password.
      </p>
      {error && <p class="error" role="alert">{error}</p>}
      {code && (
        <>
          <div
            class="qr"
            role="img"
            aria-label="Sign-in code as a QR code"
            // The SVG is made by this app from the server's own address.
            dangerouslySetInnerHTML={{ __html: code.svg }}
          />
          <p class="fingerprint" data-testid="sign-in-link">{code.link}</p>
          <button type="button" onClick={copy}>{copied ? "Copied" : "Copy link"}</button>
        </>
      )}
    </section>
  );
}
