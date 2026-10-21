// Formspree capture shared by the landing page and the audience pages (R44.1,
// R44.16). Plain fetch, no SDK: these pages have no runtime deps. Every payload
// is { email, interest, source, ...extra }; the processor is disclosed in
// privacy.html - keep the two in sync when fields change.
const FORMSPREE_ID = "mdeozqge";
const EMAIL_RE = /^[^\s@]+@[^\s@]+\.[^\s@]+$/;

export type Interest = "hosted" | "team" | "updates" | "download";

const THANKS: Record<Interest, string> = {
  hosted: "Thanks — you're on the hosted waitlist.",
  team: "Thanks — we'll be in touch about Team.",
  updates: "Thanks — we'll email you product updates.",
  download: "Thanks — we'll email you the download link.",
};

// Landing form: the entry point that led to it decides the interest.
let landingInterest: Interest = "updates";
document.addEventListener("click", (e) => {
  const a = (e.target as Element | null)?.closest<HTMLElement>("[data-interest]");
  if (a?.dataset.interest) landingInterest = a.dataset.interest as Interest;
});

type Field = HTMLInputElement | HTMLSelectElement | HTMLTextAreaElement;

function field(id: string): Field | null {
  return document.getElementById(id) as Field | null;
}

// Returns an error message, or null on success.
async function post(payload: Record<string, string>): Promise<string | null> {
  try {
    const res = await fetch(`https://formspree.io/f/${FORMSPREE_ID}`, {
      method: "POST",
      headers: { "Content-Type": "application/json", "Accept": "application/json" },
      body: JSON.stringify(payload),
    });
    if (res.ok) return null;
    const data = (await res.json().catch(() => ({}))) as { errors?: { message: string }[] };
    return data.errors?.map((e) => e.message).join(" ") || "Something went wrong — try again.";
  } catch {
    return "Couldn't reach the signup service — check your connection and retry.";
  }
}

function setInvalid(el: Element | null, on: boolean): void {
  if (on) el?.setAttribute("aria-invalid", "true");
  else el?.removeAttribute("aria-invalid");
}

interface FormIds { form: string; email: string; msg: string; source: string }

// One-field waitlist form. `fixed` pins the interest (audience pages);
// otherwise the landing's click-tracked interest is used.
export function initWaitlistForm(ids: FormIds, fixed?: Interest): void {
  document.getElementById(ids.form)?.addEventListener("submit", async (e) => {
    e.preventDefault();
    const el = field(ids.email);
    const msg = document.getElementById(ids.msg);
    const email = el?.value.trim() ?? "";
    if (!EMAIL_RE.test(email)) {
      if (msg) msg.textContent = "Enter a valid email address.";
      setInvalid(el, true);
      return;
    }
    setInvalid(el, false);
    if (msg) msg.textContent = "Sending…";
    const interest = fixed ?? landingInterest;
    const err = await post({ email, interest, source: ids.source });
    if (err) { if (msg) msg.textContent = err; setInvalid(el, true); return; }
    if (msg) msg.textContent = THANKS[interest];
    if (el) el.value = "";
  });
}

// Team quote request: required company, seats, deployment, email; optional note.
const QUOTE_REQUIRED: [id: string, error: string][] = [
  ["q-company", "Enter your company name."],
  ["q-seats", "Enter the number of seats (a whole number, 1 or more)."],
  ["q-deploy", "Choose a deployment."],
  ["q-email", "Enter a valid email address."],
];

function quoteFieldBad(id: string, v: string): boolean {
  if (!v) return true;
  if (id === "q-email") return !EMAIL_RE.test(v);
  if (id === "q-seats") return !(Number.isInteger(Number(v)) && Number(v) >= 1);
  return false;
}

export function initQuoteForm(ids: { form: string; msg: string; source: string }): void {
  document.getElementById(ids.form)?.addEventListener("submit", async (e) => {
    e.preventDefault();
    const msg = document.getElementById(ids.msg);
    const val = (id: string): string => field(id)?.value.trim() ?? "";
    let firstBad = "";
    for (const [id, text] of QUOTE_REQUIRED) {
      const bad = quoteFieldBad(id, val(id));
      setInvalid(field(id), bad);
      if (bad && !firstBad) { firstBad = id; if (msg) msg.textContent = text; }
    }
    if (firstBad) { field(firstBad)?.focus(); return; }
    if (msg) msg.textContent = "Sending…";
    const err = await post({
      email: val("q-email"), interest: "team", source: ids.source,
      company: val("q-company"), seats: val("q-seats"), deployment: val("q-deploy"), note: val("q-note"),
    });
    if (err) { if (msg) msg.textContent = err; return; }
    if (msg) msg.textContent = THANKS.team;
    (e.target as HTMLFormElement).reset();
  });
}

export function initEmailCapture(): void {
  initWaitlistForm({ form: "capture-form-2", email: "email2", msg: "capture2-msg", source: "roadmap" });
}
