import { message } from "@tauri-apps/plugin-dialog";
import { askpassReply, type Question } from "./api";

/** Dialog wording for a question from ssh. */
export function describeQuestion(q: Question): { title: string; note: string | null; accept: string } {
  if (!q.secret) {
    return { title: `${q.alias}: confirm`, note: null, accept: "Accept" };
  }
  return {
    title: `${q.alias}: password needed`,
    note: q.retry ? "That was not accepted. Try again." : null,
    accept: "OK",
  };
}

// One dialog at a time; questions from other tabs wait their turn.
let queue: Promise<void> = Promise.resolve();

/** Shows `q` after any earlier question and sends the answer back to ssh. */
export function showQuestion(q: Question): Promise<void> {
  queue = queue.then(async () => {
    const { answer, save } = await ask(q);
    try {
      await askpassReply(q.id, answer, save);
    } catch (e) {
      // ssh still got the answer; only saving it failed.
      await message(String(e), { title: "Password not saved", kind: "warning" });
    }
  });
  return queue;
}

/**
 * Resolves to the typed secret, "" for an accepted confirmation, or null if cancelled (Escape),
 * and whether the user asked to save the password.
 */
function ask(q: Question): Promise<{ answer: string | null; save: boolean }> {
  const { title, note, accept } = describeQuestion(q);
  const dialog = document.createElement("dialog");
  dialog.className = "conflict askpass";
  const form = document.createElement("form");
  form.method = "dialog";
  const heading = document.createElement("h2");
  heading.textContent = title;
  // ssh's own wording, e.g. "user@host's password:" or the host key fingerprint.
  const prompt = document.createElement("pre");
  prompt.textContent = q.prompt.trim();
  form.append(heading, prompt);
  if (note) {
    const warning = document.createElement("p");
    warning.className = "askpass-retry";
    warning.textContent = note;
    form.append(warning);
  }
  if (q.error) {
    const problem = document.createElement("p");
    problem.className = "askpass-retry";
    problem.textContent = `Saved password: ${q.error}`;
    form.append(problem);
  }
  let input: HTMLInputElement | null = null;
  if (q.secret) {
    input = document.createElement("input");
    input.type = "password";
    input.autocomplete = "off";
    input.spellcheck = false;
    input.autofocus = true;
    form.append(input);
  }
  let save: HTMLInputElement | null = null;
  if (q.secret) {
    // Off by default: saving is the user's explicit choice.
    save = document.createElement("input");
    save.type = "checkbox";
    const label = document.createElement("label");
    label.className = "askpass-save";
    label.append(save, " Save password in Windows Credential Manager");
    form.append(label);
  }
  const actions = document.createElement("div");
  actions.className = "conflict-actions";
  for (const [value, label] of [
    ["accept", accept],
    ["cancel", "Cancel"],
  ]) {
    const b = document.createElement("button");
    b.type = "submit";
    b.value = value;
    b.textContent = label;
    // A host key is accepted only by a deliberate click, not by Enter.
    b.autofocus = q.secret ? false : value === "cancel";
    actions.append(b);
  }
  form.append(actions);
  dialog.append(form);
  document.body.append(dialog);
  dialog.showModal();
  return new Promise((resolve) => {
    dialog.addEventListener(
      "close",
      () => {
        const accepted = dialog.returnValue === "accept";
        resolve({ answer: accepted ? (input?.value ?? "") : null, save: accepted && !!save?.checked });
        if (input) input.value = "";
        dialog.remove();
      },
      { once: true },
    );
  });
}
