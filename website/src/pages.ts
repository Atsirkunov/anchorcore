// Entry for the audience pages (/team, /hosted - R44.16): nav menu + forms only.
import { initNavMenu } from "./nav";
import { initThemeSwitch } from "./themeMode";
import { initQuoteForm, initWaitlistForm } from "./capture";

initNavMenu();
initThemeSwitch();
initQuoteForm({ form: "quote-form", msg: "quote-msg", source: "team-page" });
initWaitlistForm({ form: "hosted-form", email: "h-email", msg: "hosted-msg", source: "hosted-page" }, "hosted");
