---
name: data-analysis
description: Statistical analysis, trend detection, anomaly detection, visualization design. Activate for analytics, statistics, trends, anomalies, charts, metrics, dashboard data tasks.
user-invocable: true
---

# Data Analysis

## When to Activate
- User mentions: analyze, statistics, trends, anomalies, correlations, distributions, metrics, dashboard, chart, visualization, A/B test, significance
- Working with datasets, CSVs, database queries, or API response data
- Evaluating experiment results or performance metrics

## Analysis Framework

### Phase 1: Data Understanding
1. **Shape**: rows, columns, types, nulls, duplicates
2. **Distribution**: mean, median, mode, std dev, skewness for each numeric column
3. **Cardinality**: unique values per categorical column
4. **Temporal**: time range, frequency, gaps, seasonality
5. **Quality**: missing values, outliers (>3 sigma), impossible values

### Phase 2: Exploratory Analysis
| Question | Technique |
|----------|-----------|
| What's the central tendency? | Mean, median, mode |
| How spread is the data? | Std dev, IQR, range |
| Are there outliers? | Z-score >3, IQR × 1.5 |
| Is there a trend? | Moving average, linear regression |
| Is there seasonality? | Decomposition, autocorrelation |
| Are variables related? | Correlation matrix, scatter plots |
| Are groups different? | t-test, chi-square, ANOVA |

### Phase 3: Statistical Testing
**A/B Test Significance:**
- Minimum sample: 100 conversions per variant
- Minimum duration: 2 full weeks (capture weekly patterns)
- Significance level: p < 0.05 (95% confidence)
- Effect size: practically meaningful, not just statistically significant

**Common tests:**
| Scenario | Test |
|----------|------|
| Compare 2 means (normal) | Independent t-test |
| Compare 2 means (non-normal) | Mann-Whitney U |
| Compare proportions | Chi-square / Fisher's exact |
| Compare 3+ groups | ANOVA / Kruskal-Wallis |
| Correlation | Pearson (linear) / Spearman (monotonic) |
| Time series trend | Augmented Dickey-Fuller |

### Phase 4: Visualization Design
**Chart Selection:**
| Data Type | Best Chart |
|-----------|-----------|
| Trend over time | Line chart |
| Category comparison | Bar chart (horizontal if >5 categories) |
| Distribution | Histogram / box plot |
| Part of whole | Stacked bar (NOT pie charts) |
| Correlation | Scatter plot |
| Multiple dimensions | Small multiples / faceted charts |

**Rules:**
- Start y-axis at zero for bar charts (never truncate)
- Use consistent color palette across related charts
- Label axes, include units
- Annotate key events on time series
- Never use 3D charts
- Mobile-friendly: min 14px font, touch targets >44px

### Phase 5: Output
```
## Analysis: [Dataset/Question]

### Summary Statistics
[Key numbers, distributions]

### Key Findings
1. [Finding with evidence]
2. [Finding with confidence interval]
3. [Finding with comparison to baseline]

### Anomalies / Concerns
[Outliers, data quality issues, confounding variables]

### Recommendations
[Actionable next steps based on findings]

### Methodology
[Tests used, sample sizes, assumptions, limitations]
```

## Domain-Specific Patterns

### YouTube Analytics
- **CTR**: benchmark 4-6%, segment by traffic source
- **AVD (Avg View Duration)**: compare to video length, flag <40% retention
- **Impression-to-View**: measure by thumbnail variant
- **Subscriber conversion**: views-to-subs ratio by content type
- **RPM**: segment by niche, season (Q4 = peak for finance)

### Trading/Quant Analytics
- **Returns**: log returns for statistical properties
- **Sharpe ratio**: risk-adjusted returns (>1.0 good, >2.0 excellent)
- **Drawdown**: max peak-to-trough decline
- **Win rate vs profit factor**: don't optimize win rate alone
- **Regime detection**: rolling volatility, correlation breaks

## Guardrails
- NEVER claim causation from correlation alone
- NEVER cherry-pick time ranges to show desired results
- ALWAYS report sample size and confidence intervals
- ALWAYS check for confounding variables
- Apply Zero Cognitive Bias Protocol — especially confirmation bias on hypotheses
