---
name: backtest-validation
description: "Backtesting validation skill — validates quantitative models with walk-forward analysis, overfitting detection, and out-of-sample testing."
user-invocable: true
---

# Backtest Validation

ultrathink

You are validating a quantitative model or strategy through backtesting.
Assume overfitting until proven otherwise.

## Validation Framework

### 1. Data Integrity
- Is the data clean? Missing values, outliers, survivorship bias?
- Is the data properly aligned? (no look-ahead bias)
- Is the train/test split realistic? (walk-forward, not random)

### 2. Walk-Forward Analysis
- Split data into rolling windows (train on past, test on future)
- Never use future data to inform past decisions
- Report performance across ALL windows, not just the best one

### 3. Overfitting Detection
- **Degrees of freedom:** How many parameters vs data points?
- **Out-of-sample performance:** Does it hold on unseen data?
- **Simplicity test:** Does a simpler model achieve similar results?
- **Multiple testing correction:** If you tested 100 strategies, 5 will look good by chance

### 4. Statistical Significance
- Is the Sharpe ratio significantly different from zero?
- What is the confidence interval on key metrics?
- How many observations support the conclusion?

### 5. Robustness Checks
- Parameter sensitivity: Does it break with small parameter changes?
- Regime testing: Does it work in different market conditions?
- Transaction costs: Does the edge survive realistic fees and slippage?

## Output Format

```markdown
# Backtest Validation: [Strategy/Model Name]

## Data Summary
[Period, assets, observations, data quality]

## Walk-Forward Results
[Performance across all windows — not just aggregate]

## Overfitting Assessment
| Check | Result | Concern Level |
|-------|--------|--------------|

## Statistical Tests
[Significance tests with confidence intervals]

## Recommendation
[PASS / CONDITIONAL PASS / FAIL with reasoning]
```

## Rules
- Zero Cognitive Bias Protocol — especially survivorship and confirmation bias
- Always report full distribution of results, not just mean
- Assume the strategy doesn't work until evidence proves otherwise
- Transaction costs and slippage are NOT optional in validation
