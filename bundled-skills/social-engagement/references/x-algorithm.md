# How X ranks posts, and what to do about it

Source: X open-sourced its ranking code at github.com/xai-org/x-algorithm. This summary
paraphrases one creator's reading of it (Ryan, @ryangrowthh, "The X Algorithm Explained,
In Depth", 2026-11-03). It is an interpretation, not X's documentation. The weights change
without notice, so treat the ratios as direction, not law.

## The pipeline

1. **Candidates.** About 2,000 posts per feed load come from three sources: accounts you
   follow, a model that predicts what you'll like from accounts you don't follow, and
   posts popular in your interest clusters.
2. **Scoring.** For each viewer, a model predicts every action (reply, share, like, follow,
   scroll past, mute and so on) and sums them with weights.
3. **Filtering.** Drops posts older than about 48 hours, posts the viewer has already seen,
   and replies and reposts from accounts the viewer doesn't follow. Spam and low-reputation
   accounts are dropped too.

## What the weights say

- Shares (copy link, DM) count the most. A reply or quote counts about 7.5 times a like.
  A like is one of the weakest positive signals.
- Profile clicks count for nothing.
- Mutes, blocks, "not interested" and reports cost many times what a reply earns.
- Posting a lot means not every post gets a chance.
- Accounts you don't follow are discounted, but small accounts get a boost on fresh posts.

## So, for a small account

- Write things people want to answer or pass on: a real fix, a number, a before and after.
- Stay on posts under 48 hours old. Older threads only notify the person you reply to.
- A reply to someone who doesn't follow you mostly reaches that person, so be the most
  useful reply they get.
- Answer everyone who replies to you. Threads that keep going score again.
- Never risk negative signals: no spam, no duplicates, no fighting, no off-topic politics.
- One idea per post, aimed at one kind of person, with a clear takeaway.
- Credibility comes from who engages with you, so build a network of real peers in the
  niche, not follower counts.
- Don't post copy that is 100% machine-written. A person reviews and edits it.
