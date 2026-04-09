---
name: aeo-optimization
description: Answer Engine Optimization — make content rank in AI-generated answers (ChatGPT, Perplexity, Google SGE). Activate for AEO, AI search, answer engine, cited by AI, AI visibility tasks.
user-invocable: true
---

# Answer Engine Optimization (AEO)

## When to Activate
- User mentions: AEO, AI search, answer engine, cited by AI, AI visibility, Perplexity, SGE, AI overview
- Optimizing content to be cited by AI systems
- Building documentation or content sites

## AEO vs Traditional SEO
| Aspect | SEO (Ranked) | AEO (Cited) |
|--------|-------------|-------------|
| Goal | Top 10 on Google SERP | Cited in AI-generated answer |
| Format | Keyword-optimized page | Self-contained answer blocks |
| Structure | H1/H2/meta tags | Definition blocks, step lists, comparison tables |
| Authority | Backlinks, domain rating | Structured data, clear attribution, /llms.txt |
| Measurement | Rankings, CTR, traffic | Citation frequency, referral from AI |

## Three Pillars

### 1. Extractability (Can AI find and parse your answer?)
- **Self-contained answer blocks**: Each section should answer one question completely in 2-3 sentences
- **Definition blocks**: "What is X?" → First sentence IS the definition
- **Step-by-step lists**: "How to X?" → Numbered steps, each one clear
- **Comparison tables**: "X vs Y?" → Table with clear dimensions
- **FAQ sections**: Direct question → direct answer format
- **Code blocks**: Properly formatted, copy-pasteable

### 2. Authority (Should AI trust your answer?)
- **Author attribution**: Named author with credentials
- **Publication date**: Always visible and current
- **Sources**: Link to primary sources, studies, official docs
- **Structured data**: Schema.org markup (FAQ, HowTo, Article)
- **/llms.txt**: Create this file at your domain root — tells AI crawlers what content is available and how to cite it

### 3. Freshness (Is your answer current?)
- **Last updated date**: Visible on every page
- **Regular updates**: AI prefers recently updated content
- **Version-specific content**: Include year/version ("Python 3.12", "React 19")
- **Deprecation notices**: Mark outdated content clearly

## /llms.txt Standard
```
# /llms.txt — AI content index
# See: https://llmstxt.org

> This site provides [description]

## Documentation
- [Page Title](url): Brief description of what this page covers

## API Reference
- [Endpoint docs](url): REST API documentation

## Guides
- [Tutorial name](url): Step-by-step guide for X
```

## Measurement
- Search for your brand/product in ChatGPT, Perplexity, Gemini
- Track referral traffic from AI platforms in analytics
- Monitor "AI Overview" citations in Google Search Console
- Use Perplexity's citation API if available

## Guardrails
- AEO complements SEO, it does NOT replace it
- Optimizing for AI citation ≠ keyword stuffing for bots
- Quality and accuracy matter more than format tricks — AI evaluates content quality
- Apply Zero Cognitive Bias — don't over-optimize for AI at the expense of human readers
