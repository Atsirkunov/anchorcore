// tweakable copy — single source, no CMS
const copy = {
  personal: {
    title: "Your knowledge, finally together.",
    sub: "Your notes, PDFs, Jira tickets and decisions in one searchable memory. Ask like you remember it — get the source.",
    proof: "300-page PDF? Page 250 still cited. Your second brain, not another chatbot.",
    roadmap: "personal"
  },
  team: {
    title: "Run on your infrastructure — one container.",
    sub: "Self-hosted in one command. Your data stays with you, same cited answers for the whole team. Deploy via Docker, no cloud required.",
    proof: "One Docker container — your team's memory, on your infra. Who owns billing? Cited, not guessed.",
    roadmap: "team"
  },
  hosted: {
    title: "Hosted — coming soon.",
    sub: "We host it for you, same private memory, no setup. Join waitlist for early access.",
    proof: "No Docker, no infra — just connect and ask. Your memory, hosted.",
    roadmap: "hosted"
  }
} as const;

type Track = keyof typeof copy;

function setTrack(t: Track) {
  const c = copy[t];
  const title = document.getElementById("hero-title");
  const sub = document.getElementById("hero-sub");
  const proof = document.getElementById("track-proof");
  const roadmap = document.getElementById("roadmap-track");
  const cta = document.getElementById("cta-primary") as HTMLAnchorElement | null;
  const note = document.getElementById("cta-note");
  if (title) title.textContent = c.title;
  if (sub) sub.textContent = c.sub;
  if (proof) proof.textContent = c.proof;
  if (roadmap) roadmap.textContent = c.roadmap;
  if (cta) {
    if (t === "team") {
      cta.textContent = "Deploy via Docker — Self-hosted";
      cta.href = "https://github.com/Atsirkunov/anchorcore#hosting";
    } else if (t === "hosted") {
      cta.textContent = "Join waitlist — Hosted (soon)";
      cta.href = "#roadmap";
    } else {
      cta.textContent = "Download — Free while in validation";
      cta.href = "https://github.com/Atsirkunov/anchorcore/releases";
    }
  }
  if (note) {
    if (t === "team") note.innerHTML = "Docker • one container • your infra • data stays in your VPC";
    else if (t === "hosted") note.innerHTML = "Coming soon — we host it, no setup · <code>hosted</code> tab";
    else note.innerHTML = "macOS & Windows • one file, no Docker • data stays in <code>~/.anchorcore</code>";
  }

  document.querySelectorAll<HTMLButtonElement>(".track-toggle .toggle").forEach(b => {
    const isActive = b.dataset.track === t;
    b.classList.toggle("active", isActive);
    b.setAttribute("aria-selected", String(isActive));
  });

  // persist for app hint (?track=) + reload
  try { localStorage.setItem("anchorcore.track", t); } catch {}
  const url = new URL(location.href);
  url.searchParams.set("track", t);
  history.replaceState(null, "", url.toString());
}

function initTrack() {
  const params = new URL(location.href).searchParams.get("track") as Track | null;
  const stored = (() => { try { return localStorage.getItem("anchorcore.track") as Track | null; } catch { return null; } })();
  const valid: Track[] = ["personal", "team", "hosted"];
  const initial: Track = (params && valid.includes(params) ? params : null) ?? (stored && valid.includes(stored) ? stored : null) ?? "personal";
  // default to personal for faster validation (design-system.md:128)
  setTrack(initial);
  document.querySelectorAll<HTMLButtonElement>(".track-toggle .toggle").forEach(b => {
    b.addEventListener("click", () => setTrack(b.dataset.track as Track));
  });
}

// tiny capture — no backend yet, just local + mailto hint
function captureEmail(inputId: string, msgId: string) {
  const el = document.getElementById(inputId) as HTMLInputElement | null;
  const msg = document.getElementById(msgId);
  const email = el?.value.trim() ?? "";
  if (!email || !email.includes("@")) {
    if (msg) msg.textContent = "Enter a valid work email.";
    return;
  }
  const track = (() => { try { return localStorage.getItem("anchorcore.track") ?? "personal"; } catch { return "personal"; } })();
  try {
    const key = `anchorcore.capture:${track}`;
    const prev = JSON.parse(localStorage.getItem(key) || "[]");
    prev.push({ email, at: new Date().toISOString(), track });
    localStorage.setItem(key, JSON.stringify(prev));
  } catch {}
  if (msg) msg.textContent = `Thanks — ${track} waitlist saved locally. Hook this to your email service.`;
  if (el) el.value = "";
}

// expose for inline onclick
declare global { interface Window { capture: () => void; capture2: () => void; } }
window.capture = () => captureEmail("email", "capture-msg");
window.capture2 = () => captureEmail("email2", "capture2-msg");

initTrack();
