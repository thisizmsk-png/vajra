---
name: frontend-architecture
description: Frontend architecture, design quality, component patterns inspired by Apple/Linear/Vercel/Stripe/OpenAI/Anthropic. Activate for frontend, UI, landing page, dashboard, component architecture, design quality tasks.
user-invocable: true
---

# Frontend Architecture & Design Quality

> Build interfaces that look like they belong on Apple.com, not a hackathon demo.

## When to Activate
- User mentions: frontend, UI, landing page, dashboard, component, layout, responsive, design quality
- Building any user-facing page or component
- Reviewing UI code for design quality
- Creating a new web application frontend

## Design Quality Checklist (Before Shipping ANY UI)

### Typography
- [ ] Max 2 font families (1 sans-serif + 1 mono)
- [ ] Headings: negative letter-spacing (-0.02em to -0.04em)
- [ ] Body: 16-18px base, 1.6-1.7 line-height
- [ ] Overlines: 11-12px uppercase, positive letter-spacing (+0.08em)
- [ ] No more than 3 font weights used on any single page
- [ ] Contrast ratio >= 4.5:1 for body text, >= 3:1 for large text

### Spacing
- [ ] 4px base grid (all spacing divisible by 4)
- [ ] Section gaps: 96-160px (generous, not cramped)
- [ ] Card padding: 24-32px
- [ ] Component internal spacing: 12-16px
- [ ] Max content width: 1280px (max-w-7xl)
- [ ] Text content max: 640px (max-w-2xl) for readability

### Color (Dark Theme)
- [ ] Background: 3-level depth (primary, secondary, tertiary)
- [ ] Text: 3 levels (primary, secondary, tertiary)
- [ ] One brand color (used sparingly — CTAs, links, highlights)
- [ ] One accent color (used very sparingly — badges, indicators)
- [ ] Semantic colors: success/warning/error/info
- [ ] Never pure black (#000) or pure white (#fff) — use near-black/near-white

### Layout Patterns (From Top Sites)

**Hero Section (Apple/Vercel pattern):**
```
[overline badge — small, pill-shaped]
[headline — 48-72px, bold, gradient or white]
[subheadline — 18-20px, muted color, max-w-2xl]
[CTA input or buttons — centered]
[trust signal — small text below CTA]
```

**Stats Bar (Stripe pattern):**
```
[number — large, bold, brand color] [label — small, muted]
[number] [label]  |  [number] [label]  |  [number] [label]
```

**Problem Section (Linear pattern):**
```
[section heading — centered]
[3 cards in a row, each with icon + heading + paragraph]
[cards have subtle border, hover state, consistent height]
```

**Feature Bento Grid (PostHog pattern):**
```
[2-column or 3-column grid]
[each cell: icon + heading + description]
[varying cell sizes for visual interest]
[subtle border, bg-secondary fill]
```

**Pricing Section (Stripe pattern):**
```
[3 columns: Starter | Pro (highlighted) | Agency]
[Pro card: "Most Popular" badge, brand border, slight elevation]
[Feature list with check icons]
[CTA button at bottom of each card]
```

**CTA Section (Apple pattern):**
```
[large heading — "Start today" style, centered]
[short supporting text]
[single prominent CTA button]
[trust text below: "No credit card required"]
```

### Component Patterns

**Cards:**
```css
rounded-2xl border border-border bg-bg-secondary p-6
hover:border-border-hover transition-colors
```

**Buttons (Primary):**
```css
rounded-xl bg-brand px-6 py-3 text-sm font-medium text-white
shadow-lg shadow-brand/20
hover:bg-brand-hover hover:shadow-brand-hover/30
transition-all
```

**Buttons (Secondary):**
```css
rounded-xl border border-border bg-bg-secondary px-6 py-3
text-sm font-medium text-text-primary
hover:bg-bg-tertiary transition-colors
```

**Input Fields:**
```css
rounded-xl border border-border bg-bg-secondary px-5 py-3.5
text-base text-text-primary placeholder-text-tertiary
outline-none focus:border-brand focus:ring-2 focus:ring-brand/30
transition-all
```

**Badge/Pill:**
```css
rounded-full border border-border bg-bg-tertiary px-4 py-1.5
text-xs font-medium text-text-secondary
```

### Animation Rules
- Default transition: 150ms ease-out
- Hover effects: transform + opacity (never color alone)
- Scroll reveals: fade-up with 20px translate, stagger 50ms per item
- Loading states: skeleton screens, not spinners (except inline actions)
- Never animate layout properties (width, height, top, left) — use transform

### Dashboard-Specific Patterns

**Score Gauges (SVG rings):**
- Radius = (size - strokeWidth) / 2
- `strokeDasharray` for progress
- Glow effect via `filter: drop-shadow(0 0 8px color)`
- Animate with CSS transition on mount

**Data Grid (Breakdown):**
- 3-column responsive grid
- Each cell: label + progress bar + score
- Color-coded: green (>=70%), amber (>=40%), red (<40%)

**Issue List:**
- Grouped by severity: Critical → Warning → Info
- Expandable with recommendation + suggested fix
- Impact dots (1-10 scale)
- "Upgrade for more" upsell at bottom

### Page Structure (12-Section Standard)

For a SaaS landing page, follow this order:
1. **Nav** — sticky, transparent → solid on scroll
2. **Hero** — headline + scanner/CTA
3. **Social proof** — stats bar or logo wall
4. **Problem** — 3 pain points
5. **Solution** — features grid (bento)
6. **How it works** — 3 numbered steps
7. **Testimonials** — if available
8. **Pricing** — 3 tiers
9. **FAQ** — expandable
10. **Final CTA** — simple, one button
11. **Footer** — 4 columns + social links
12. **Scroll to top** — optional

### Responsive Breakpoints
- Mobile: < 640px (stack everything, 16px padding)
- Tablet: 640-1024px (2-column grids, 24px padding)
- Desktop: > 1024px (full layout, 32px+ padding)
- Use `sm:`, `md:`, `lg:` Tailwind prefixes consistently

## Anti-AI-Slop Protocol (MANDATORY)

Every UI you produce MUST pass these checks. AI-generated interfaces have a recognizable "slop aesthetic" — avoid it.

### The 7 Anti-Slop Tenets

**1. Design Thinking Before Code**
- Define purpose, audience, and tone BEFORE writing JSX
- Ask: "What will make this memorable?" — if you can't answer, you're building slop
- Commit to a bold directional choice (organic/natural, luxury/refined, technical/precise)

**2. Identify and Reject These Specific Slop Patterns**
- Generic fonts (default Inter/Arial without customization)
- Purple gradient backgrounds (the #1 AI slop signal)
- Cookie-cutter 3-card layouts with identical padding
- Every element having the same shadow depth
- Scattered micro-interactions (random bouncing, spinning, easing)
- Stock-photo hero sections
- More than 2 primary CTAs visible at once

**3. Typography as Identity**
- Choose distinctive, characterful fonts — not just Inter
- Letter spacing is a signal system:
  - `-0.01em` (tight): Hero headings — confident, premium feel
  - `0.01em` (barely open): Body text — comfortable reading
  - `0.04em` (open): Badge text — reads as label
  - `0.08em` (wide): Small uppercase labels — reads as category
- Each text level must feel intentionally different

**4. Color with Conviction**
- Commit to a BOLD color decision — no hedging with safe grays
- Dominant color + sharp accent > flat uninspired palette
- Use CSS variables for cohesive theming
- Every color must serve a communicative purpose

**5. Motion Justifies Its Existence**
- Card entrance: orients perception ("this just arrived")
- Staggered reveal: prevents "wall of content" effect
- Hover lift: confirms interactivity (tactile feedback)
- Button press: physical interaction feedback
- NEVER: random bouncing, spinning icons, gratuitous transitions

**6. Shadow Hierarchy**
- Different shadow scales for different component levels
- Cards ≠ buttons ≠ modals — each gets appropriate depth
- Shadows communicate 3D spatial relationships, not decoration
- AI slop: every element gets identical `shadow-md`

**7. Atmospheric Depth**
- Use gradients, textures, and layered backgrounds — not flat colors
- Create depth through transparency and blur (glassmorphism when appropriate)
- Background should feel like an environment, not a painted wall

### The Slop Detector Test
Before shipping, ask these 5 questions:
1. Does this look like every other AI-generated landing page? → Redesign
2. Could I identify the brand from the UI alone (no logo)? → If no, add character
3. Is every card/section the same visual weight? → Add hierarchy
4. Are there any decorative elements that don't aid comprehension? → Remove
5. Would a designer look at this and say "AI made this"? → Fix the tells

## Process (When Building a New Page)

1. **Structure first** — lay out the 12 sections with placeholder text
2. **Typography hierarchy** — set all heading sizes, body sizes, colors
3. **Spacing pass** — ensure consistent gaps between all sections
4. **Component pass** — style each card, button, input consistently
5. **Responsive pass** — test at 375px, 768px, 1280px
6. **Animation pass** — add transitions and hover states
7. **Accessibility pass** — keyboard navigation, ARIA labels, contrast
8. **Performance pass** — lazy load images, minimize JS, check Core Web Vitals

## Guardrails
- NEVER use more than 2 font families
- NEVER ship without checking mobile layout
- NEVER use inline styles — use design tokens via CSS custom properties
- NEVER add decorative elements that don't improve comprehension
- Apply Zero Cognitive Bias Protocol — don't copy a pattern because it's trendy, use it because it serves the user
