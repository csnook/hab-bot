import { SERVER_SEES } from "./join";

export function ServerSees() {
  return (
    <details class="sees">
      <summary>What your server can see</summary>
      <ul>
        {SERVER_SEES.map((line) => (
          <li key={line}>{line}</li>
        ))}
      </ul>
    </details>
  );
}
