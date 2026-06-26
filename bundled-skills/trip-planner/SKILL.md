---
name: trip-planner
description: Plan a complete, realistic multi-stop road/air trip end to end, then ship it as a beautiful shareable PDF with tappable Google Maps links. Use for trip planning, itineraries, vacation/road-trip routing, "plan our trip", multi-city day-by-day schedules, and travel logistics. Geography-first, red-teamed, date-verified.
user-invocable: true
---

# trip-planner

Turn a loose set of wishes ("fly into LA, do Universal, then Vegas") into a tight, day-by-day itinerary that actually survives contact with reality, then deliver it as a polished, shareable PDF the whole group can navigate from their phones.

Built from a real 7-person SoCal + Vegas plan. The hard-won lesson: **the trip breaks on geography, dates, and logistics, not on the list of fun things.** This skill front-loads those.

## When to use
Any "plan a trip / itinerary / road trip / vacation" request, single or multi-city, with cars, flights, or both. Also for auditing an existing plan for failure modes.

## Communication style
Be brutal and unhedged. Name the tradeoffs and biases out loud, give a recommendation (not a survey), and quantify (drive times, dollars, buffers). If a stated plan is geographically or logistically broken, say so plainly and propose the fix.

---

## The method (run the phases in order)

### Phase 1. Pin the hard constraints before planning anything
Get these explicitly; do not assume:
- **Dates** (and convert to days-of-week; it decides what's open).
- **Party size** + **vehicle** (and whether luggage actually fits, see Phase 3).
- **Flights in and out**: airports + **times**. The return flight + its time is usually the single biggest constraint.
- **Lodging location, the EXACT spot**, not just the city. This is the hidden variable that dictates every drive. Geocode it.
- **Budget tier**, and any must-dos vs nice-to-haves.

A login-gated booking link (Airbnb, etc.) usually 403s on fetch; just ask the user for the neighborhood / nearest landmark.

### Phase 2. Geography first (this is where plans are won or lost)
- Locate the **base** and compute rough drive times to every cluster of attractions.
- **Surface immediately if the base is far from the sights.** (Real example: a "Los Angeles" trip whose Airbnb was near Shandin Hills Golf Club, actually San Bernardino, ~70 mi EAST of LA. Every LA sight became a 1.5 to 2 hr commute; Vegas/San Diego got closer.) The base reshapes the whole trip.
- **Front-load** attractions that are only reachable on one day (e.g., the airport-arrival day is often the only time you're on the "right" side of a sprawl); stack the far-side icons there.
- Kill backtracks: order stops so each day flows one direction. Merge a "beach morning + city evening" into a single directional sweep, not a there-and-back.
- For each long leg, give a realistic range, not the Google-best-case (holiday traffic, mountain passes, desert gaps).

### Phase 3. Red-team the plan (always, before delivering)
Spawn an adversarial pass (the `duryodhana` red-team agent, or a disciplined self-critique) and return **P0 (trip-breaking) / P1 (serious) / P2 (annoyance)** with concrete fixes. The recurring failure modes:
- **Worst-traffic days.** Holiday return days are landmines. (Jul 5, Sunday after July 4: Vegas→LA's worst day of the year; a 4.5 hr drive becomes 7 to 9 hr. If a fixed flight sits on the far end, the departure time is *load-bearing*, name it.)
- **Perishable bookings.** Holiday hotels and dated theme-park/club tickets sell out and spike. List what must be booked *today*, with all-in pricing (resort fees, parking, surcharges), not headline rates.
- **Vehicle vs luggage.** A 12-passenger van seats 12 people but has ~no cargo room; N people + N bags for a week need an *extended* SUV (Suburban/Yukon XL/Expedition MAX) or two vehicles. Confirm cubic feet, not seat count. Always 2 named drivers for long legs; request gas not EV for desert routes; keep collision/roadside coverage; carry water.
- **Late-night vs early-departure collisions.** A big party night that bumps against a fixed early departure is a missed-flight risk. Resolve it explicitly: designate rested/sober drivers, hard departure time, pre-pack the night before.
- **Single-vehicle desert breakdown** equals whole group stranded; favor redundancy or take the coverage seriously.
- **Arrival-time / unknowns** that quietly add 1 to 2 hr/day. Flag the assumptions.

### Phase 4. Verify open + hours for the ACTUAL dates (non-negotiable)
The user's rule: **always check that places are open on the specific dates.** Web-search every operator-run or timed place; do not assume:
- **Day-of-week closures** (e.g., many museums close Mon or Wed; LACMA is closed Wednesdays; Griffith Observatory closed Mondays). Outdoor installations (Urban Light, Seven Magic Mountains) stay accessible even when the parent museum is closed.
- **Seasonal / annual maintenance closures** (e.g., Palm Springs Tramway closes ~Sep to Oct).
- **Holiday hours** and reduced **summer heat hours** (desert zoos may close ~1:30 PM).
- **Heat & season.** Desert in July hits 100 to 110°F: dawn/dusk only, no midday hikes; pivot to **mountains** (~30°F cooler) for a full active day. Always add a hydration/sun note.
Cite the source links you used so the user can re-check.

### Phase 5. Build the deliverable (a shareable PDF with map links)
Default output is a **one-to-three-page PDF** the group can tap to navigate. Two build paths (prefer the first):

1. **Designed (HTML to headless Chrome).** Best typography + clickable links. Write a self-contained HTML file, render with:
   ```
   "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome" --headless --disable-gpu \
     --run-all-compositor-stages-before-draw --virtual-time-budget=3000 \
     --print-to-pdf="OUT.pdf" "file:///path/itin.html"
   ```
   Use the bundled `assets/itinerary_template.html` as the starting structure. `<a href>` links become tappable PDF annotations.
2. **Simple (reportlab fallback)** when no browser is available, Platypus tables, `<a href>` for links. Less pretty, fully portable.

**Every stop gets a Google Maps link; every long drive gets a directions link:**
- Place: `https://www.google.com/maps/search/?api=1&query=<URL-encoded name>`
- Route: `https://www.google.com/maps/dir/?api=1&origin=<A>&destination=<B>`

Save to the user's Desktop (or chosen folder) and send the file. Verify pages + clickable-link count with `pypdf` before claiming done.

### Phase 6. Iterate
Expect rapid edits (party size, swapped days, new stops, recolor). Keep **one source of truth per variant** and re-render all variants together so they never drift. When you derive a color variant, generate it from the primary HTML via string-replace of the palette so content stays in sync.

---

## PDF design spec (taste rules, applied via design-taste-frontend discipline)
- **One locked accent** for the whole document; type-driven hierarchy, not boxes everywhere.
- **Zero em-dashes or en-dashes** anywhere visible (use hyphens). This is the #1 AI tell.
- Big day numerals (01-05) in the accent, a tabular time rail, a route strip, a base-note callout, and two alert cards: a filled "can't-slip" card and an outlined "book-now" card.
- Avoid the AI-default beige+brass palette. Ship at least two variants on request.
- Font: a clean system sans (Avenir Next / SF), not Inter-by-default.

**Palette variants used (light ivory paper, near-black ink, one accent):**
- *Coral ("designed")*: accent `#DB4F36`, tint `#FBEAE4`, line `#F0C9BE`.
- *Emerald & Gold ("lavish")*: accent `#0C6B4E`, gold detail `#A8842F` (hero rule + label only), tint `#E4F0EA`, line `#BBD6CA`.

`mcp__visualize__show_widget` is also great for an inline interactive itinerary card alongside the PDF.

---

## Hard checklist (don't ship without these)
- [ ] Base location pinned and its drive-time tax surfaced.
- [ ] Day-by-day flows one direction; no silent backtracks.
- [ ] Red-team P0/P1/P2 delivered with fixes; the load-bearing constraints named.
- [ ] Every place verified open for its real date (+ day-of-week + season + heat).
- [ ] Perishable bookings listed with all-in pricing.
- [ ] PDF built, links verified clickable, saved + sent.

## Related
Pairs with `design-taste-frontend` (PDF aesthetics), the `duryodhana` red-team agent, and `mcp__visualize` for inline widgets. Driven under the `[[vajra]]` harness.
