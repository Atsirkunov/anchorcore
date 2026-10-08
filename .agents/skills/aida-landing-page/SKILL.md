---
name: aida-landing-page
description: Build a conversion-focused marketing landing page using the AIDA blueprint with premium dark-mode Tailwind styling.
---

# AIDA Landing Page

Use this skill when asked to create, rewrite, or review a product marketing
landing page. Structure first, style second.

## 1. Page structure (AIDA)

Build these five sections in order, one per screen region:

1. **Hero (Attention):** bold headline stating the primary *benefit* (not
   features), one short sub-headline, one app-screenshot placeholder, one
   clear primary button (`Get Started` / `Download` / `Sign up`).
2. **Problem/Solution grid (Interest):** 3 visual columns, each naming one
   pain point the product removes and the outcome.
3. **Feature showcase (Desire):** 2–3 alternating rows (image left, text
   right, then flipped) showing the product in action.
4. **Social proof / pricing (Trust):** placeholders for 2–3 testimonials
   and/or one simple pricing card.
5. **Final call to action (Action):** one focused footer section with a
   single download/signup action, no competing links.

## 2. Design constraints

Single HTML file with Tailwind CSS:

1. **Color:** dark mode base (Slate-900 background), one vibrant accent
   (e.g. Emerald-500 or Indigo-500) reserved strictly for buttons and focus
   elements.
2. **Typography:** clean modern sans-serif; heavy weight for headers
   (`font-black`), muted mid-contrast for descriptions
   (e.g. `text-slate-400`).
3. **Layout:** generous whitespace (`py-24`/`py-32` sections, `gap-8`–`gap-12`
   grids) so the page breathes.
4. **Visual anchors:** every screenshot/image slot is a styled placeholder
   container (subtle `border-slate-800`, smooth gradient) mimicking a real UI
   viewport — never an empty `<img>` or blank box.
5. **Copy:** benefit-driven headlines written for the product at hand, no
   lorem-ipsum or generic filler.

## 3. Brand override

If the project already has a design system (tokens, palette, fonts), its
tokens win over section 2 defaults. Apply the AIDA structure with the
project's own colors and type, and note each substitution explicitly instead
of silently restyling the brand.
