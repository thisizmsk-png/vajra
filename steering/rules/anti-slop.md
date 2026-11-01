# Steering: Anti-Slop (BLOCKING)

- id: anti-slop
- domain: frontend
- blocking: true
- file-patterns: ["**/*.tsx", "**/*.jsx", "**/*.vue", "**/*.svelte", "**/*.html", "**/*.css", "**/*.scss"]

These patterns make an interface read as AI-generated. A violation blocks merge
the same as a security finding; it is not a style preference. Only the project
owner can waive a rule, in writing, for one named project.

## Never ship (blocking)

Color and surface:
1. Decorative gradients.
2. Purple and black as the palette.
3. Rainbow coloring.
4. Neon colors.
5. Basic pastel colors.
6. Pure white backgrounds. Use a tinted neutral.
7. Liquid glass or glassmorphism.
8. A drop shadow on everything.
9. Soft, uniform corner radius on everything.
10. A colored accent stripe on the edge of a card or callout.

Layout:
11. Three feature cards in a row.
12. Bento grids.
13. Three pricing tiers as the default.
14. Decorative terminal windows.

Decoration:
15. Radial orbs or glowing blobs.
16. Dot grid backgrounds.
17. Sparkle icons.
18. Loose decorative icons beside text.
19. Checkmark bullets.
20. Emojis in the interface.

Type:
21. Inter, Geist or Space Grotesk.

Motion:
22. Animated arrows.
23. Hover animation on everything. Animate only what the hover explains.

Copy:
24. Em dashes.
25. The "It's not X, it's Y" construction.
26. Fake testimonials, logos or numbers.

## Always ship (blocking)

27. Real product demos, never mock screens or stock placeholders.
28. Skeleton loaders wherever content loads asynchronously.
29. Terms of Service on every public site.
30. A Privacy Policy on every public site.

## Review

Quote the rule number and the offending line. Rules 6, 21 and 24 can be checked
by searching the diff; the rest need a look at the rendered page.
