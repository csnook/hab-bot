import { render } from "preact";
import { App } from "./App";
import { AlarmWindow } from "./AlarmWindow";
import { alarmIdOf } from "./alarm";
import "./style.css";

// An alarm window loads this same page with ?alarm=<occurrence id>.
const alarm = alarmIdOf(window.location.search);
render(
  alarm ? <AlarmWindow occurrenceId={alarm} /> : <App />,
  document.getElementById("app")!,
);
