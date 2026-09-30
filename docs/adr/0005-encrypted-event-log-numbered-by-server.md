# Sync is an encrypted event log per reminder list, numbered by the server

Every change is an event: made on a device, signed by it, and encrypted with its reminder list's key (ADR 0002). Each list has its own stream of events, and the server adds each event to its stream with the next number, without being able to read it. Devices build the current state by applying a stream's events in order, with their own unsent events applied on top until the server numbers them. The first claim in a stream wins, the stream is the history, and a standalone device numbers its own streams. We chose this over CRDT libraries because the data is small and structured with no text to merge, the server already has to order claims and key changes, and the encryption layers built for CRDTs haven't reached 1.0.

## Considered Options

- **CRDT documents per list (Automerge or Loro)**, with encrypted updates relayed by the server: edits merge for free, but claims would still need the server's order and the history an attributed record of its own, so there would be two mechanisms. Loro's encryption extension is at 0.x and Automerge's encryption stack is pre-alpha.
- **A hybrid**: an event log for occurrences and history, and a CRDT for reminder settings. Two mechanisms again, for little gain.
- **Server-authoritative sync from a database** (PowerSync, Electric): the server has to read the data it syncs.

## Consequences

- We write the merge rules ourselves, with no outside tests that devices converge. For a setting changed on two devices out of touch, the change made last wins, by a hybrid logical clock; the server rejects clocks far ahead of its own, and the history keeps the losing change. Actions on occurrences follow their own rules, such as a completion beating a skip.
- Concurrent edits to a reminder's note don't merge: the later one wins, and the earlier one stays in the history.
- The server sees each event's stream, device, size and time received, but nothing inside it.
- Every encrypted or signed item carries an identifier of the algorithms used, so they can be changed later without breaking what's already stored.
