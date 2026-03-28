# Ephemera: Trust & Safety Implementation Spec

**Parent document:** [ARCHITECTURE.md](./ARCHITECTURE.md)
**Companion:** [05_moderation_safety.md](./05_moderation_safety.md) (design philosophy & data structures)
**Version:** 1.0
**Date:** 2026-03-27
**Status:** Implementation-ready specification

---

## 0. Problem Statement

All moderation primitives exist as standalone code:
- `ephemera-abuse`: `RateLimiter`, `ReputationScore`, `FingerprintStore`
- `ephemera-mod`: `ContentFilter`, `ReportService`, `LocalBlocklist`

**None of them are wired into the system.** Posts flow through `gossip_ingest.rs` and `PostService::create` with zero moderation checks. An attacker can flood the network with spam, CSAM, harassment campaigns, or coordinated false reports with no resistance whatsoever.

This document is the implementation spec that closes that gap.

---

## 1. Threat Catalog

Every threat is rated on two axes:
- **Severity**: How much harm does a successful attack cause? (Critical / High / Medium / Low)
- **Likelihood**: How easy is it to execute given current defenses? (Very High / High / Medium / Low)

### 1.1 CSAM Distribution

| Attribute | Value |
|-----------|-------|
| Severity | **Critical** |
| Likelihood | **High** (no media scanning wired) |
| Attacker profile | Predators using the platform's anonymity and lack of scanning |
| Victim impact | Child exploitation; platform faces criminal liability and immediate shutdown |
| Current defense | `CsamFilter` spec exists in `05_moderation_safety.md`; bloom filter not built |
| Required defense | Client-side perceptual hash check in media pipeline BEFORE encryption; relay-side verification for public content; silent rejection with generic error; automated NCMEC reporting pipeline |

**This is the existential threat.** A single confirmed CSAM distribution event on a platform with no scanning gets the project killed -- legally, reputationally, and morally. This is non-negotiable priority zero.

### 1.2 Porn / NSFL Spam Flooding

| Attribute | Value |
|-----------|-------|
| Severity | **High** |
| Likelihood | **Very High** (no content filter wired, no rate limiting wired) |
| Attacker profile | Bots posting adult content to overwhelm the public feed |
| Victim impact | Users see unwanted graphic content; platform becomes unusable; drives away legitimate users |
| Current defense | `ContentFilter` exists but is not called anywhere; heuristic checks are text-only |
| Required defense | Rate limiting on post creation; reputation gating (no media during warming); content sensitivity labels; user-controlled filter levels; SimHash deduplication to catch copy-paste floods |

### 1.3 Harassment Campaigns (Brigading)

| Attribute | Value |
|-----------|-------|
| Severity | **High** |
| Likelihood | **High** |
| Attacker profile | Coordinated group targeting a single user with mass replies, DMs, or reports |
| Victim impact | Psychological harm; target driven off platform; if combined with doxxing, real-world safety threat |
| Current defense | Block/mute spec exists in `05_moderation_safety.md`; not implemented in gossip or services |
| Required defense | Per-thread reply rate limits; DM rate limits for strangers; block propagation to gossip layer; coordinated-action detection (multiple new accounts targeting same user) |

### 1.4 Bot Networks / Sybil Attacks

| Attribute | Value |
|-----------|-------|
| Severity | **High** |
| Likelihood | **Medium** (PoW exists but warming not enforced) |
| Attacker profile | Automated identity creation to overwhelm rate limits, stuff votes, or amplify content |
| Victim impact | Legitimate users' content drowned out; moderation quorum corrupted; reputation system gamed |
| Current defense | PoW for identity creation exists; `ReputationScore` warming period exists but is not checked |
| Required defense | Wire reputation gating into `PostService::create` and `gossip_ingest_loop`; adaptive PoW difficulty for identity surges; graph analysis for suspiciously dense disconnected clusters |

### 1.5 Commercial Spam and Scam Links

| Attribute | Value |
|-----------|-------|
| Severity | **Medium** |
| Likelihood | **Very High** (no spam detection wired) |
| Attacker profile | Marketers, cryptocurrency scammers, phishing operators |
| Victim impact | Platform unusable; users lose money to scams; reputational damage |
| Current defense | `FingerprintStore` (SimHash) exists but is not called; `ContentFilter` exists but is not called |
| Required defense | Wire SimHash check into both gossip ingest and local post creation; URL pattern detection; rate limiting; reputation-weighted feed ranking |

### 1.6 Report Weaponization (False Flagging)

| Attribute | Value |
|-----------|-------|
| Severity | **High** |
| Likelihood | **High** (no anti-weaponization logic exists) |
| Attacker profile | Political operatives, personal grudge holders, trolls who enjoy silencing others |
| Victim impact | Innocent users' content removed; chilling effect on speech; platform becomes a censorship tool |
| Current defense | `ReportService` prevents duplicate reports from the same reporter, but has no threshold logic, no reporter accountability, no false-report detection |
| Required defense | **This is the hardest problem. See Section 4 (dedicated section).** |

### 1.7 Identity Impersonation

| Attribute | Value |
|-----------|-------|
| Severity | **Medium** |
| Likelihood | **Medium** |
| Attacker profile | Someone copying another user's display name, avatar, and posting style |
| Victim impact | Reputation damage to the impersonated user; social engineering attacks on their contacts |
| Current defense | Handles exist (`ephemera-social/src/handle.rs`) with uniqueness enforcement |
| Required defense | Handle reservation tied to identity key; visual indicators for connection age; report category for impersonation with lower threshold |

### 1.8 Doxxing

| Attribute | Value |
|-----------|-------|
| Severity | **Critical** |
| Likelihood | **Medium** |
| Attacker profile | Someone posting another person's real-world identity information (address, phone, employer) |
| Victim impact | Real-world safety threat; harassment escalation; potential physical harm |
| Current defense | None |
| Required defense | Report category with expedited review; community tombstone with lower quorum for confirmed doxxing; pattern detection for structured PII (phone numbers, addresses); content hash blocklist for confirmed doxxing content |

### 1.9 Gore / Violence Content

| Attribute | Value |
|-----------|-------|
| Severity | **Medium** |
| Likelihood | **Medium** |
| Attacker profile | Shock content posters; terroristic threats with accompanying imagery |
| Victim impact | Psychological trauma for unwitting viewers; platform reputation damage |
| Current defense | `ReportReason::Violence` exists but triggers no action |
| Required defense | Content sensitivity labels (self-applied and community-applied); user-configurable filter level; report-to-label pipeline (not binary block) |

### 1.10 Coordinated Inauthentic Behavior

| Attribute | Value |
|-----------|-------|
| Severity | **High** |
| Likelihood | **Low** (platform is small, but grows with adoption) |
| Attacker profile | State actors, influence operations, astroturfing campaigns |
| Victim impact | Public discourse manipulation; erosion of trust in the platform |
| Current defense | None |
| Required defense | Graph analysis for coordinated posting patterns; temporal correlation detection (many accounts posting similar content within short windows); EigenTrust local scoring to deprioritize low-trust clusters |

---

## 2. Defense Layers

Each defense layer specifies: the existing code to wire, where to wire it, what happens on failure, and how false positives are mitigated.

### 2.1 Rate Limiting

**Existing code:** `ephemera_abuse::RateLimiter` in `crates/ephemera-abuse/src/rate_limit.rs`

**Wire points:**

#### 2.1.1 Local Post Creation (`PostService::create`)

```
BEFORE building the post:
  1. Look up the author's ReputationScore
  2. Determine the effective rate limit:
     - Warming period: capacity=1, refill_rate=1/3600 (1 post/hour)
     - Normal (rep < 10): capacity=5, refill_rate=5/3600 (5 posts/hour)
     - Established (rep >= 10): capacity=10, refill_rate=10/3600 (10 posts/hour)
  3. Call rate_limiter.check(&author_identity, ActionType::Post)
  4. On Err(RateLimited): return error with retry_after_secs to caller
  5. On Ok: proceed to build post
```

**Implementation location:** `crates/ephemera-node/src/services/post.rs`, at the top of `PostService::create()` and `PostService::create_and_publish()`.

**New code needed:**
- `PostService` needs access to a shared `RateLimiter` and `ReputationScore` store (passed via `ServiceContainer`).
- Add `ActionType::Report` to the rate limiter enum for report rate limiting.

**False-positive mitigation:** Rate limits are generous for established users (10 posts/hour burst). The `retry_after_secs` value tells the client exactly when to retry. No content is lost -- just delayed.

#### 2.1.2 Gossip Ingest (`gossip_ingest_loop`)

```
AFTER deserializing and validating the post, BEFORE storing:
  1. Call rate_limiter.check(&post.author, ActionType::Post)
  2. On Err(RateLimited):
     - Log at TRACE level (don't spam our own logs)
     - Record ReputationEvent::RateLimitViolation for the author
     - Drop the post (do not store)
     - Continue to next message
  3. On Ok: proceed to store
```

**Implementation location:** `crates/ephemera-node/src/gossip_ingest.rs`, after line 65 (after `validate_post` succeeds).

**False-positive mitigation:** Peer-side rate limiting uses 2x the stated limit (as specified in `05_moderation_safety.md` Section 9.3). A post arriving at 11/hour is accepted with a warning; a post at 21/hour is rejected. This tolerance accounts for clock skew and network batching.

#### 2.1.3 Report Rate Limiting

```
BEFORE accepting a report:
  1. Call rate_limiter.check(&reporter, ActionType::Report)
     - Configuration: capacity=5, refill_rate=5/3600 (5 reports/hour)
  2. On Err(RateLimited): return error "You are filing reports too quickly"
  3. On Ok: proceed to create_report
```

**Implementation location:** New `ActionType::Report` variant in `crates/ephemera-abuse/src/rate_limit.rs`. Check added before `ReportService::create_report()` call.

**Why this matters:** Without report rate limiting, a single attacker can flood the moderation system with thousands of reports, overwhelming moderators and DoS-ing the review pipeline. Five reports per hour is generous for legitimate use (how often does a real user encounter five separate pieces of abusive content in an hour?) but prevents automated report flooding.

### 2.2 Reputation Gating

**Existing code:** `ephemera_abuse::ReputationScore` and `ephemera_abuse::Capability` in `crates/ephemera-abuse/src/reputation.rs`

**Wire points:**

#### 2.2.1 Local Post Creation

```
BEFORE building the post:
  1. Load the author's ReputationScore from the reputation store
  2. If post has media attachments:
     - Check reputation.has_capability(Capability::AttachPhotos)
     - On false: return error "Photos require reputation >= 5 (your reputation: X)"
  3. If warming period is active:
     - Restrict to text-only content
     - Apply warming rate limit (1/hour)
  4. Record ReputationEvent::PostCreated on successful post creation
```

**Implementation location:** `crates/ephemera-node/src/services/post.rs`

#### 2.2.2 Gossip Ingest

```
AFTER rate limit check, BEFORE storing:
  1. Load the author's ReputationScore
  2. If author reputation value < -50:
     - DROP the post (do not store, do not propagate)
     - Log at DEBUG: "Dropped post from globally toxic identity {author}"
  3. If post has media and author lacks AttachPhotos capability:
     - DROP the post
  4. On successful store: record ReputationEvent::PostCreated
```

**Implementation location:** `crates/ephemera-node/src/gossip_ingest.rs`

**False-positive mitigation:** The -50 threshold is severe -- it requires multiple confirmed community tombstones. A single false report cannot push anyone to -50. The warming period applies only to brand-new identities and automatically ends after 7 days of normal use.

#### 2.2.3 Reputation Store

**New code needed:** A `ReputationStore` that persists `ReputationScore` per `IdentityKey`.

```rust
// New file: crates/ephemera-node/src/services/reputation.rs
pub struct ReputationStore {
    scores: HashMap<IdentityKey, ReputationScore>,
}

impl ReputationStore {
    pub fn get_or_create(&mut self, identity: &IdentityKey) -> &mut ReputationScore;
    pub fn record_event(&mut self, identity: &IdentityKey, event: ReputationEvent);
    pub fn apply_daily_decay(&mut self);
}
```

**Persistence:** Serialize to SQLite on shutdown; load on startup. Decay is applied lazily on access (compare `last_decay_at` with current time).

### 2.3 Content Filtering

**Existing code:** `ephemera_mod::ContentFilter` in `crates/ephemera-mod/src/filter.rs`

**Wire points:**

#### 2.3.1 Local Post Creation

```
AFTER building the post, BEFORE storing:
  1. Extract text body from post.content.text_body()
  2. Call content_filter.check_text(text)
  3. Match result:
     - FilterResult::Allow -> proceed to store
     - FilterResult::Block(reason) -> return error to caller with reason
     - FilterResult::RequireReview(reason) -> store with review_pending=true flag;
       do NOT publish to gossip until review clears
```

**Implementation location:** `crates/ephemera-node/src/services/post.rs`, after `builder.build()` succeeds.

#### 2.3.2 Gossip Ingest

```
AFTER rate limit and reputation checks, BEFORE storing:
  1. Extract text body from deserialized post
  2. Call content_filter.check(&post_content_id, text)
  3. Match result:
     - FilterResult::Allow -> proceed to store
     - FilterResult::Block(reason) -> drop post, log at TRACE
     - FilterResult::RequireReview(reason) -> store with review_pending flag
```

**Implementation location:** `crates/ephemera-node/src/gossip_ingest.rs`

**False-positive mitigation:**
- `RequireReview` does NOT remove content. It flags it for moderation quorum review. Content remains visible to the author and their connections until a quorum acts.
- Excessive caps detection has a high threshold (80% uppercase) and minimum length (20 chars), so short exclamations like "WOW!" or "OMG" are not flagged.
- The blocklist is hash-based (exact match or perceptual hash match), so there is no fuzzy false-positive risk on blocklist checks.

### 2.4 SimHash Spam Detection

**Existing code:** `ephemera_abuse::FingerprintStore` in `crates/ephemera-abuse/src/fingerprint.rs`

**Wire points:**

#### 2.4.1 Gossip Ingest (Primary Defense)

```
AFTER content filter check, BEFORE storing:
  1. Extract text body from post
  2. Call fingerprint_store.check_and_record(text)
  3. If returns true (near-duplicate detected):
     - Check author reputation:
       * If rep < 10: DROP post (low-rep spam)
       * If rep >= 10: store but flag as potential_duplicate=true
     - Record ReputationEvent::RateLimitViolation for the author
  4. If returns false (unique content): proceed to store
```

**Implementation location:** `crates/ephemera-node/src/gossip_ingest.rs`

#### 2.4.2 Local Post Creation

```
BEFORE publishing to gossip:
  1. Call fingerprint_store.check_duplicate(text)
  2. If duplicate detected:
     - Warn the user: "This looks similar to a recent post. Post anyway?"
     - If user confirms: proceed (it may be an intentional repost)
     - This is a CLIENT-SIDE check, not a hard block
```

**Implementation location:** Client-side (Tauri command layer), not in `PostService` itself. The user should have agency over their own posts.

**False-positive mitigation:** SimHash with Hamming distance <= 3 has a low false-positive rate for text longer than a few words. Very short posts ("hello", "lol") may collide, but the 20-character minimum for text checks mitigates this. Established users (rep >= 10) get their duplicate posts stored with a flag rather than dropped -- the system trusts them more.

### 2.5 Content Sensitivity Labels

**New code needed.** This is the key alternative to binary content removal.

```rust
// New file: crates/ephemera-mod/src/sensitivity.rs

/// Content sensitivity level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SensitivityLabel {
    /// Safe for all audiences.
    General,
    /// May contain mature themes (political, controversial).
    Mature,
    /// Contains nudity or sexual content.
    Nsfw,
    /// Contains graphic violence or disturbing imagery.
    Graphic,
}

/// How a sensitivity label was applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LabelSource {
    /// Author self-labeled the content.
    AuthorApplied,
    /// Community applied via report consensus.
    CommunityApplied,
    /// Automated heuristic detection.
    HeuristicDetected,
}

/// User's personal filter preference.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SensitivityPreference {
    /// Show General content (always true).
    pub show_general: bool,
    /// Show Mature content (default: true).
    pub show_mature: bool,
    /// Show NSFW content (default: false).
    pub show_nsfw: bool,
    /// Show Graphic content (default: false).
    pub show_graphic: bool,
}
```

**Wire points:**
- Post creation: allow author to attach a `SensitivityLabel` to their post.
- Feed display: filter posts based on user's `SensitivityPreference`.
- Report outcome: instead of removing disagreeable-but-legal content, apply a community sensitivity label.

**Why this matters for false reports:** Content that is offensive but not illegal (political speech, controversial art, unpopular opinions) gets labeled, not removed. Users who don't want to see it can filter it. Users who do want to see it can. This eliminates the entire category of "I disagree with this" reports being weaponized into censorship.

### 2.6 Gossip-Level Filtering

**Wire points in `gossip_ingest_loop`:**

The complete filtering pipeline for incoming gossip posts, in order:

```
1. Deserialize post from payload             [EXISTING - line 53]
2. Validate signature, PoW, TTL             [EXISTING - line 65]
3. Check local blocklist (user-blocked authors) [NEW]
   -> If author is blocked by local user: drop silently
4. Rate limit check                          [NEW - Section 2.1.2]
   -> If over 2x limit: drop + reputation penalty
5. Reputation gate                           [NEW - Section 2.2.2]
   -> If author rep < -50: drop
   -> If media without capability: drop
6. Content filter                            [NEW - Section 2.3.2]
   -> If blocked: drop
   -> If requires review: store with flag
7. SimHash duplicate check                   [NEW - Section 2.4.1]
   -> If duplicate from low-rep author: drop
8. Blocklist hash check                      [NEW]
   -> If content hash is in blocklist: drop
9. Store post                                [EXISTING - line 75]
10. Emit PostReceived event                  [EXISTING - line 86]
11. Record ReputationEvent::PostCreated      [NEW]
```

**Implementation:** Refactor `gossip_ingest_loop` to call a new `fn filter_incoming_post(...)` that runs steps 3-8 and returns `FilterDecision::Accept | FilterDecision::AcceptWithFlag(String) | FilterDecision::Reject(String)`.

**New struct for passing shared state:**

```rust
// In crates/ephemera-node/src/gossip_ingest.rs

pub struct IngestContext {
    pub rate_limiter: Arc<Mutex<RateLimiter>>,
    pub reputation_store: Arc<Mutex<ReputationStore>>,
    pub content_filter: Arc<ContentFilter>,
    pub fingerprint_store: Arc<Mutex<FingerprintStore>>,
    pub local_blocklist: Arc<Mutex<LocalBlocklist>>,
    pub blocked_authors: Arc<Mutex<HashSet<IdentityKey>>>,
}
```

---

## 3. Report System Design

### 3.1 Current State

`ReportService` in `crates/ephemera-mod/src/report.rs` provides:
- Report creation with deduplication (same reporter + same content = rejected)
- Report listing and counting
- Report reasons (Harassment, HateSpeech, Violence, Spam, Csam, Impersonation, Other)

**What is missing:**
- Report thresholds (how many reports trigger action?)
- Reporter weighting (whose reports count more?)
- Reporter accountability (what happens to serial false reporters?)
- Auto-tombstone logic
- Appeal mechanism
- Report-abuse detection
- Category-specific thresholds

### 3.2 Report Weighting

Not all reports are equal. A report from a 90-day account with 200 reputation carries more weight than a report from a 2-day account with 0 reputation.

```rust
// New file: crates/ephemera-mod/src/report_weight.rs

/// Compute the weight of a report based on the reporter's reputation.
pub fn report_weight(reporter_rep: &ReputationScore) -> f64 {
    let base = if reporter_rep.is_warming() {
        0.1  // Reports from warming accounts barely count
    } else if reporter_rep.value() < 10.0 {
        0.3  // Low-rep accounts get reduced weight
    } else if reporter_rep.value() < 50.0 {
        0.7  // Normal accounts get near-full weight
    } else {
        1.0  // Established accounts get full weight
    };

    // Penalty for reporters with poor track record
    // (see Section 4.3 for reporter_accuracy calculation)
    let accuracy_mult = reporter_accuracy_multiplier(reporter_rep);

    base * accuracy_mult
}

/// Reporters who file many reports that get dismissed see their
/// weight reduced. Reporters whose reports are consistently upheld
/// retain full weight.
fn reporter_accuracy_multiplier(reporter_rep: &ReputationScore) -> f64 {
    // Starts at 1.0
    // Decreases by 0.1 for each dismissed report (floor 0.1)
    // Increases by 0.05 for each upheld report (ceiling 1.0)
    // Stored as a field on ReputationScore
    1.0 // placeholder -- implementation in ReputationScore
}
```

### 3.3 Category-Specific Thresholds

Different report categories have different action thresholds because the consequences differ:

| Category | Weighted Report Threshold for Auto-Tombstone | Min Distinct Reporters | Reporter Requirements | Action |
|----------|----------------------------------------------|----------------------|----------------------|--------|
| **CSAM** | 1.0 (single credible report) | 1 | rep >= 0, age >= 0 | Immediate hash-blocklist + auto-tombstone + escalate to NCMEC pipeline |
| **Doxxing** | 2.0 | 2 | rep >= 5, age >= 3d | Auto-tombstone within 1 hour; expedited quorum review |
| **Violence/Threats** | 3.0 | 3 | rep >= 10, age >= 7d | Auto-tombstone; quorum review within 24 hours |
| **Harassment** | 4.0 | 3 | rep >= 10, age >= 7d | Auto-tombstone; quorum review within 24 hours |
| **Hate Speech** | 5.0 | 4 | rep >= 20, age >= 14d | Sensitivity label applied first; tombstone only after quorum vote |
| **Spam** | 3.0 | 3 | rep >= 5, age >= 3d | Auto-tombstone; add to SimHash blacklist |
| **Impersonation** | 3.0 | 2 | rep >= 10, age >= 7d | Auto-tombstone; notify impersonated user if identifiable |
| **Other** | 5.0 | 5 | rep >= 20, age >= 14d | No auto-action; queued for quorum review |

**Why CSAM has the lowest threshold:** The legal and moral consequences of hosting CSAM, even briefly, are catastrophic. A single credible report triggers immediate action. The risk of a false CSAM report removing legitimate content is acceptable compared to the risk of inaction on actual CSAM.

**Why "Other" and "Hate Speech" have the highest thresholds:** These are the categories most likely to be weaponized for political censorship. "I disagree with this person's politics" gets filed as "Hate Speech" or "Other". High thresholds, established-reporter requirements, and sensitivity labeling (instead of removal) protect dissenting voices.

### 3.4 Auto-Tombstone Logic

```rust
// New file: crates/ephemera-mod/src/tombstone.rs

pub struct TombstoneEvaluator {
    report_service: ReportService,
    reputation_store: ReputationStore,
}

impl TombstoneEvaluator {
    /// Evaluate whether a piece of content should be auto-tombstoned.
    ///
    /// Called whenever a new report is filed.
    pub fn evaluate(&self, content_id: &ContentId) -> TombstoneDecision {
        let reports = self.report_service.reports_for_content(content_id);
        if reports.is_empty() {
            return TombstoneDecision::NoAction;
        }

        // Group reports by category (use the most severe category reported)
        let most_severe_reason = self.most_severe_reason(&reports);
        let threshold = self.threshold_for_reason(&most_severe_reason);

        // Count weighted reports from DISTINCT, QUALIFIED reporters
        let mut seen_reporters = HashSet::new();
        let mut weighted_total = 0.0;
        let mut qualified_distinct = 0u32;

        for report in &reports {
            if seen_reporters.contains(&report.reporter) {
                continue; // Already counted this reporter
            }
            seen_reporters.insert(&report.reporter);

            let reporter_rep = self.reputation_store.get(&report.reporter);
            let Some(rep) = reporter_rep else {
                continue; // Unknown reporter, skip
            };

            // Check if reporter meets minimum requirements for this category
            if !self.reporter_qualifies(rep, &most_severe_reason) {
                continue;
            }

            let weight = report_weight(rep);
            weighted_total += weight;
            qualified_distinct += 1;
        }

        // Check both weighted total AND distinct reporter count
        if weighted_total >= threshold.weighted_threshold
            && qualified_distinct >= threshold.min_distinct_reporters
        {
            TombstoneDecision::AutoTombstone {
                reason: most_severe_reason,
                weighted_score: weighted_total,
                reporter_count: qualified_distinct,
            }
        } else {
            TombstoneDecision::NoAction
        }
    }
}

pub enum TombstoneDecision {
    NoAction,
    AutoTombstone {
        reason: ReportReason,
        weighted_score: f64,
        reporter_count: u32,
    },
}
```

### 3.5 Author Notification and Appeal

When content is auto-tombstoned:

1. The post is marked `is_tombstone = 1` in SQLite (existing mechanism in `PostService::delete`).
2. The author receives a local notification: "Your post was flagged by the community for: {reason}. It has been hidden from public feeds."
3. The author can **appeal** by republishing with a higher PoW cost (2x the normal difficulty). This signals "I believe this content is legitimate and I'm willing to pay a computational cost to prove it."
4. Appealed content enters the moderation quorum review queue.
5. If the quorum upholds the tombstone: content stays removed, author takes a -50 reputation hit (`ReputationEvent::CommunityTombstone`).
6. If the quorum reverses the tombstone: content is restored, reporters who filed false reports take a reputation penalty (see Section 4.3).

**New code needed:**
- `appeal` method on `PostService` that accepts a higher-difficulty PoW stamp.
- `AppealStatus` enum: `Pending`, `Upheld`, `Reversed`.
- SQLite column `appeal_status` on the posts table.

---

## 4. False Report Resistance

This is the hardest problem in content moderation. Every mechanism designed to remove bad content can be weaponized to remove good content. This section designs defenses against that weaponization.

### 4.1 The Attack Vectors for Report Weaponization

| Attack | Description | Who does this |
|--------|-------------|---------------|
| **Mass false reporting** | Coordinate 10+ accounts to report the same post | Political operatives, trolls, griefers |
| **Serial false reporting** | One account files hundreds of reports on different content | Grudge holders, bored trolls |
| **Category escalation** | Report political speech as "CSAM" or "Violence" to trigger lower thresholds | Sophisticated censors |
| **Moderator capture** | Get elected as moderators, then use quorum power to silence opponents | Organized factions |
| **Report-then-harass** | Report someone's post, then DM them to gloat or threaten | Personal grudge holders |

### 4.2 Detection: Coordinated False Reporting

```rust
// New file: crates/ephemera-mod/src/report_analysis.rs

/// Detect coordinated reporting campaigns.
///
/// A coordination signal exists when:
/// 1. Multiple reports on the same content arrive within a short time window
/// 2. The reporters have unusual graph properties (e.g., all created within
///    the same week, all connected to each other but not to the target)
/// 3. The reporters have a history of reporting the same targets
pub struct CoordinationDetector {
    /// Time window for detecting burst reports (default: 1 hour).
    burst_window_secs: u64,
    /// Minimum reports in the window to flag as potential coordination.
    burst_threshold: u32,
}

impl CoordinationDetector {
    /// Analyze reports on a specific content item for coordination signals.
    pub fn analyze(
        &self,
        content_id: &ContentId,
        reports: &[Report],
        reputation_store: &ReputationStore,
    ) -> CoordinationSignal {
        // Signal 1: Temporal burst
        let recent_reports: Vec<_> = reports
            .iter()
            .filter(|r| {
                let age = now_secs() - r.timestamp.as_secs();
                age < self.burst_window_secs
            })
            .collect();

        if recent_reports.len() < self.burst_threshold as usize {
            return CoordinationSignal::None;
        }

        // Signal 2: Reporter age clustering
        // If all reporters were created within the same 48-hour window,
        // this is suspicious.
        let reporter_ages: Vec<u32> = recent_reports
            .iter()
            .filter_map(|r| {
                reputation_store
                    .get(&r.reporter)
                    .map(|rep| rep.age_days())
            })
            .collect();

        let age_variance = statistical_variance(&reporter_ages);
        let young_reporters = reporter_ages.iter().filter(|&&a| a < 14).count();

        if young_reporters > reporter_ages.len() / 2 {
            return CoordinationSignal::SuspectedSybil {
                young_account_ratio: young_reporters as f64 / reporter_ages.len() as f64,
            };
        }

        if age_variance < 2.0 && reporter_ages.len() >= 3 {
            return CoordinationSignal::SuspectedCoordination {
                age_variance,
                burst_count: recent_reports.len() as u32,
            };
        }

        CoordinationSignal::None
    }
}

pub enum CoordinationSignal {
    /// No coordination detected.
    None,
    /// Suspected Sybil attack (many young accounts reporting together).
    SuspectedSybil { young_account_ratio: f64 },
    /// Suspected coordination (accounts with suspiciously similar ages).
    SuspectedCoordination { age_variance: f64, burst_count: u32 },
}
```

**When coordination is detected:**
1. The auto-tombstone threshold is DOUBLED for this content item.
2. The report is flagged for expedited quorum review with a coordination warning.
3. All reporters involved are flagged for reporter-accuracy tracking.

### 4.3 Reporter Accountability (Without Deanonymization)

Reporters remain pseudonymous, but their reporting track record affects the weight of their future reports.

```rust
// Extension to ReputationScore in crates/ephemera-abuse/src/reputation.rs

/// Reporter accuracy tracking fields (added to ReputationScore).
pub struct ReporterTrackRecord {
    /// Reports filed that were upheld by moderation quorum.
    pub upheld_reports: u32,
    /// Reports filed that were dismissed by moderation quorum.
    pub dismissed_reports: u32,
    /// Reports filed that were reversed on appeal.
    pub reversed_reports: u32,
}

impl ReporterTrackRecord {
    /// Accuracy ratio: upheld / (upheld + dismissed + reversed).
    /// Returns 1.0 if no reports have been resolved yet (benefit of the doubt).
    pub fn accuracy(&self) -> f64 {
        let total = self.upheld_reports + self.dismissed_reports + self.reversed_reports;
        if total == 0 {
            return 1.0; // No track record = benefit of the doubt
        }
        self.upheld_reports as f64 / total as f64
    }

    /// Weight multiplier based on accuracy.
    /// Accuracy >= 0.7 -> 1.0x (normal weight)
    /// Accuracy 0.5-0.7 -> 0.5x (reduced weight)
    /// Accuracy < 0.5 -> 0.2x (heavily reduced)
    /// Accuracy < 0.2 -> 0.0x (reports effectively ignored)
    pub fn weight_multiplier(&self) -> f64 {
        let acc = self.accuracy();
        if acc >= 0.7 {
            1.0
        } else if acc >= 0.5 {
            0.5
        } else if acc >= 0.2 {
            0.2
        } else {
            0.0
        }
    }
}
```

**Reputation consequences for false reporting:**
- If a moderator quorum dismisses a report: reporter gets `-1` reputation point.
- If an appeal is upheld (content restored): reporter gets `-3` reputation points.
- If a reporter's accuracy drops below 0.2: their reports are effectively ignored (weight = 0.0) until they file upheld reports.
- A reporter who files 20+ reports in a single day has all reports from that day deprioritized (queued behind normal reports for moderation review).

**Why not harder penalties?** Harsher penalties would discourage legitimate reporting. A user who sees genuine abuse should not be afraid to report it because one previous report was a judgment call. The penalties are proportional: a single dismissed report costs 1 reputation point, which is trivially recoverable. Only sustained false reporting (accuracy < 0.2) effectively disables a reporter.

### 4.4 Distinguishing "I Don't Like This" from "This Is Illegal"

The report categories are explicitly split into two tiers:

**Tier 1: Legal/Safety Reports** (lower thresholds, faster action)
- `CSAM` -- Universally illegal, automated escalation
- `Violence` -- Threats of physical harm, graphic violence
- `Doxxing` -- Real-world safety threat (new category to add)
- `Harassment` -- Targeted, repeated abuse of a specific person

**Tier 2: Community Standards Reports** (higher thresholds, labeling before removal)
- `HateSpeech` -- Offensive but potentially protected speech
- `Spam` -- Commercial/repetitive content
- `Impersonation` -- Pretending to be someone else
- `Other` -- Catch-all

**Key design decision: Tier 2 reports result in sensitivity labels, not removal, unless a moderation quorum specifically votes for removal.** This is the critical distinction that prevents political censorship:

```
User reports post as "Hate Speech"
  -> Report is filed and weighted
  -> If threshold met: SensitivityLabel::Mature applied to post
  -> Post is still visible to users whose filter allows Mature content
  -> Post is NOT removed unless a moderation quorum votes RemoveContent
  -> Moderation quorum sees blinded content (no author identity)
  -> Quorum votes:
     - Dismiss (false report) -> reporters penalized
     - LabelSensitive (label is appropriate) -> label stays
     - RemoveContent (actually violates community standards) -> tombstone
```

**This means:** Controversial political speech, unpopular religious views, edgy humor, and artistic nudity are labeled, not removed. Users who want to see everything can turn off filters. Users who want a sanitized feed can filter aggressively. Nobody gets to silence someone else just by clicking "Report."

### 4.5 Anti-Brigading: Protecting Targeted Users

When a user receives reports on multiple different posts within a short window, this triggers brigading detection:

```rust
pub struct BrigadingDetector {
    /// Time window for detecting targeted reporting (default: 24 hours).
    window_secs: u64,
    /// Number of distinct posts reported to trigger brigading detection.
    post_threshold: u32,
}

impl BrigadingDetector {
    /// Check if an author is being targeted by a coordinated reporting campaign.
    pub fn is_target(
        &self,
        author: &IdentityKey,
        report_service: &ReportService,
    ) -> bool {
        let all_reports = report_service.list_reports();
        let recent_reports_against_author: Vec<_> = all_reports
            .iter()
            .filter(|r| {
                // We need a way to look up the author of reported content.
                // This requires joining reports with post metadata.
                let age = now_secs() - r.timestamp.as_secs();
                age < self.window_secs
            })
            .collect();

        let distinct_posts: HashSet<_> = recent_reports_against_author
            .iter()
            .map(|r| &r.content_id)
            .collect();

        distinct_posts.len() >= self.post_threshold as usize
    }
}
```

**When brigading is detected:**
1. Auto-tombstone thresholds are TRIPLED for all content from the targeted author.
2. Reports against this author are flagged for expedited quorum review with a brigading warning.
3. The targeted user is notified: "We've detected a coordinated reporting campaign against your content. Your posts have additional protection while this is under review."

### 4.6 Moderator Capture Resistance

The moderation quorum design from `05_moderation_safety.md` (5-of-7 quorum, 30-day rotation) is the primary defense. Additional safeguards:

1. **Moderator diversity requirement:** No more than 3 of 7 moderators can share mutual connections. This prevents a single friend group from controlling a quorum.
2. **Moderator recusal:** If a moderator has a mutual connection with either the reporter or the reported content's author, they must recuse from that vote. (They won't know the author's identity due to blinded review, but the system can enforce this server-side.)
3. **Moderator accountability:** Moderators whose votes are consistently overruled on appeal (accuracy < 0.5 over 30 days) lose their moderator status.
4. **Quorum randomization:** For each report, 7 moderators are randomly selected from the eligible pool (not the same 7 every time). This prevents targeted corruption of a fixed quorum.

---

## 5. Implementation Plan

### Phase 1: Critical Path (Week 1-2)

These changes prevent the platform from being immediately exploited.

#### 5.1.1 Wire Rate Limiting into Post Creation

**Files to modify:**
- `crates/ephemera-node/src/services/post.rs` -- Add `RateLimiter` check at top of `create()` and `create_and_publish()`

**New files:**
- None (uses existing `RateLimiter`)

**Shared state to add:**
- `Arc<Mutex<RateLimiter>>` to `ServiceContainer` (or whatever holds shared node state)

**Tests:**
- `test_post_creation_rate_limited` -- Create 11 posts rapidly, assert 11th fails with `RateLimited` error
- `test_post_creation_warming_rate` -- Create identity, verify 1/hour limit before day 7
- `test_rate_limit_resets` -- Verify tokens refill after waiting

#### 5.1.2 Wire Rate Limiting into Gossip Ingest

**Files to modify:**
- `crates/ephemera-node/src/gossip_ingest.rs` -- Add `RateLimiter` check after `validate_post()`, use 2x tolerance

**Shared state to add:**
- Pass `Arc<Mutex<RateLimiter>>` into `gossip_ingest_loop`

**Tests:**
- `test_gossip_drops_rate_limited_post` -- Simulate rapid posts from same author over gossip
- `test_gossip_tolerates_slight_burst` -- Verify 2x tolerance works (15 posts/hour accepted when limit is 10)

#### 5.1.3 Wire Content Filter into Both Paths

**Files to modify:**
- `crates/ephemera-node/src/services/post.rs` -- Add `ContentFilter::check_text()` call
- `crates/ephemera-node/src/gossip_ingest.rs` -- Add `ContentFilter::check()` call

**Shared state to add:**
- `Arc<ContentFilter>` (ContentFilter is read-only after initialization, so no Mutex needed unless blocklist is updated at runtime -- use `Arc<RwLock<ContentFilter>>` if runtime blocklist updates are needed)

**Tests:**
- `test_blocklisted_content_rejected` -- Add hash to blocklist, attempt to post matching content
- `test_excessive_caps_flagged_for_review` -- Post ALL CAPS content, verify it gets review flag
- `test_normal_content_passes` -- Verify clean content is not filtered

#### 5.1.4 Wire SimHash into Gossip Ingest

**Files to modify:**
- `crates/ephemera-node/src/gossip_ingest.rs` -- Add `FingerprintStore::check_and_record()` call

**Shared state to add:**
- `Arc<Mutex<FingerprintStore>>` to ingest context

**Tests:**
- `test_duplicate_post_dropped` -- Post same text twice via gossip, verify second is dropped
- `test_unique_posts_pass` -- Post different text, verify both stored
- `test_duplicate_from_established_user_flagged_not_dropped` -- High-rep user posts duplicate, verify stored with flag

### Phase 2: Reputation System (Week 2-3)

#### 5.2.1 Create ReputationStore

**New files:**
- `crates/ephemera-node/src/services/reputation.rs`

**Implementation:**
- `HashMap<IdentityKey, ReputationScore>` with SQLite persistence
- Lazy decay on access (track `last_decay_at` per identity)
- Methods: `get_or_create`, `record_event`, `get_value`, `has_capability`

**Tests:**
- `test_new_identity_starts_warming` -- New identity has warming=true, rep=0
- `test_warming_ends_after_seven_days` -- set_age_days(7) ends warming
- `test_reputation_gates_media` -- Warming identity cannot attach photos
- `test_reputation_persists_across_restart` -- Save to SQLite, reload, verify same values

#### 5.2.2 Wire Reputation into Post Creation

**Files to modify:**
- `crates/ephemera-node/src/services/post.rs` -- Check warming period and capabilities before building post

**Tests:**
- `test_warming_identity_cannot_post_media` -- New identity tries to post with attachment, gets rejected
- `test_warming_identity_limited_to_one_per_hour` -- New identity can post once, second post in same hour rejected
- `test_established_identity_full_rate` -- Identity with rep >= 10 can post 10/hour

#### 5.2.3 Wire Reputation into Gossip Ingest

**Files to modify:**
- `crates/ephemera-node/src/gossip_ingest.rs` -- Check author reputation before storing

**Tests:**
- `test_toxic_identity_dropped` -- Identity with rep < -50 has posts dropped
- `test_warming_identity_media_dropped` -- Warming identity's media posts dropped at gossip level

### Phase 3: Report System (Week 3-4)

#### 5.3.1 Add Report Rate Limiting

**Files to modify:**
- `crates/ephemera-abuse/src/rate_limit.rs` -- Add `ActionType::Report` variant with capacity=5, refill_rate=5/3600

**Tests:**
- `test_report_rate_limiting` -- File 6 reports, verify 6th is rate-limited
- `test_report_rate_refills` -- File 5 reports, wait for refill, file another

#### 5.3.2 Implement Report Weighting

**New files:**
- `crates/ephemera-mod/src/report_weight.rs`

**Tests:**
- `test_warming_reporter_low_weight` -- Report from warming account has weight 0.1
- `test_established_reporter_full_weight` -- Report from rep=50 account has weight 1.0
- `test_inaccurate_reporter_reduced_weight` -- Reporter with accuracy < 0.5 has weight multiplied by 0.2

#### 5.3.3 Implement Auto-Tombstone Evaluator

**New files:**
- `crates/ephemera-mod/src/tombstone.rs`

**Tests:**
- `test_csam_single_report_tombstones` -- Single CSAM report triggers tombstone
- `test_spam_needs_three_reports` -- Spam report needs 3 distinct qualified reporters
- `test_hate_speech_labels_first` -- Hate speech reports apply label, not tombstone, until quorum votes
- `test_unqualified_reporters_ignored` -- Reports from accounts not meeting minimum requirements are not counted

#### 5.3.4 Implement Reporter Accountability

**Files to modify:**
- `crates/ephemera-abuse/src/reputation.rs` -- Add `ReporterTrackRecord` fields

**New files:**
- `crates/ephemera-mod/src/report_analysis.rs` -- Coordination detection

**Tests:**
- `test_dismissed_report_reduces_reporter_rep` -- Reporter loses 1 point when report dismissed
- `test_reversed_appeal_penalizes_reporter` -- Reporter loses 3 points when appeal reverses tombstone
- `test_serial_false_reporter_ignored` -- Reporter with accuracy < 0.2 has weight 0.0
- `test_coordination_detected` -- 5 reports from accounts all created same week triggers coordination signal

### Phase 4: Sensitivity Labels (Week 4-5)

#### 5.4.1 Implement Sensitivity System

**New files:**
- `crates/ephemera-mod/src/sensitivity.rs`

**Database changes:**
- Add `sensitivity_label` column to posts table (default: 'general')
- Add `label_source` column to posts table (default: null)
- Add user preferences table for `SensitivityPreference`

**Tests:**
- `test_author_self_labels_nsfw` -- Author marks post as NSFW, verify label stored
- `test_community_label_applied` -- Enough reports on Tier 2 category applies label
- `test_feed_filters_by_preference` -- User with show_nsfw=false does not see NSFW posts
- `test_labeled_content_not_removed` -- Labeled content is still in storage, just filtered from feed

#### 5.4.2 Implement Appeal Flow

**Files to modify:**
- `crates/ephemera-node/src/services/post.rs` -- Add `appeal()` method

**Database changes:**
- Add `appeal_status` column to posts table
- Add `appeal_pow_hash` column for the higher-difficulty PoW stamp

**Tests:**
- `test_appeal_requires_higher_pow` -- Appeal with normal PoW rejected, appeal with 2x PoW accepted
- `test_appeal_restores_content_pending_review` -- Appealed content becomes visible again
- `test_upheld_appeal_permanent_tombstone` -- Quorum upholds tombstone, content stays removed

### Phase 5: Advanced Defenses (Week 5-6)

#### 5.5.1 Brigading Detection

**New files:**
- `crates/ephemera-mod/src/brigading.rs`

**Tests:**
- `test_brigading_detected` -- 5 different posts from same author reported in 24h triggers detection
- `test_brigading_increases_thresholds` -- When brigading detected, auto-tombstone threshold triples
- `test_author_notified_of_brigading` -- Targeted author gets notification

#### 5.5.2 Coordination Detection

**New files:**
- `crates/ephemera-mod/src/report_analysis.rs` (extends from Phase 3)

**Tests:**
- `test_sybil_reporters_detected` -- 5 reporters all created in last 3 days triggers SuspectedSybil
- `test_coordination_doubles_threshold` -- Suspected coordination doubles auto-tombstone threshold

#### 5.5.3 Gossip Propagation Filtering

**Files to modify:**
- Gossip publish logic (wherever posts are forwarded to other peers)

**Logic:**
- Do not propagate posts from authors with rep < -50
- Do not propagate posts that matched the local blocklist
- Do propagate sensitivity-labeled posts (recipients decide their own filter level)

**Tests:**
- `test_toxic_author_not_propagated` -- Post from rep < -50 author stored locally but not forwarded
- `test_blocklisted_post_not_propagated` -- Blocklisted post dropped, not forwarded

---

## 6. Complete Gossip Ingest Pipeline (Reference)

After all phases are implemented, `gossip_ingest_loop` processes each incoming post through this pipeline:

```
Incoming gossip message
    |
    v
[1] Deserialize as Post
    |-- Err: log trace, continue
    |-- Ok: proceed
    |
    v
[2] Validate signature, PoW, TTL (validate_post)
    |-- Err: log warn, continue
    |-- Ok: proceed
    |
    v
[3] Local author blocklist check
    |-- Blocked: drop silently, continue
    |-- Not blocked: proceed
    |
    v
[4] Rate limit check (2x tolerance)
    |-- Over 2x limit: drop, record RateLimitViolation, continue
    |-- Over 1x limit: accept with warning flag
    |-- Under limit: proceed
    |
    v
[5] Reputation gate
    |-- Author rep < -50: drop, log debug, continue
    |-- Author warming + has media: drop, continue
    |-- Author lacks required capability: drop, continue
    |-- Ok: proceed
    |
    v
[6] Content filter (blocklist + heuristics)
    |-- Block: drop, log trace, continue
    |-- RequireReview: proceed with review_pending flag
    |-- Allow: proceed
    |
    v
[7] SimHash near-duplicate check
    |-- Duplicate + author rep < 10: drop, record RateLimitViolation, continue
    |-- Duplicate + author rep >= 10: proceed with duplicate flag
    |-- Unique: proceed
    |
    v
[8] Store post in content store + metadata DB
    |-- Err: log warn, continue
    |-- Ok: proceed
    |
    v
[9] Record ReputationEvent::PostCreated for author
    |
    v
[10] Emit PostReceived event
    |
    v
[11] Forward to peers (unless author rep < -50 or blocklisted)
```

---

## 7. Complete Post Creation Pipeline (Reference)

After all phases are implemented, `PostService::create` processes each local post through this pipeline:

```
User calls create post
    |
    v
[1] Load author's ReputationScore
    |
    v
[2] Reputation capability check
    |-- Warming + has media: return error "Photos require reputation >= 5"
    |-- Lacks required capability: return error with explanation
    |-- Ok: proceed
    |
    v
[3] Rate limit check (exact limit, no tolerance)
    |-- RateLimited: return error with retry_after_secs
    |-- Ok: proceed
    |
    v
[4] Build post (PostBuilder, sign, compute PoW)
    |-- Err: return build error
    |-- Ok: proceed
    |
    v
[5] Content filter check on text body
    |-- Block: return error with reason
    |-- RequireReview: store with review_pending flag, do NOT publish to gossip
    |-- Allow: proceed
    |
    v
[6] Store post in content store + metadata DB
    |-- Err: return storage error
    |-- Ok: proceed
    |
    v
[7] Record ReputationEvent::PostCreated for author
    |
    v
[8] If create_and_publish: publish to gossip network
    |
    v
[9] Return success response to caller
```

---

## 8. Database Schema Changes

### 8.1 Posts Table Extensions

```sql
ALTER TABLE posts ADD COLUMN sensitivity_label TEXT DEFAULT 'general';
ALTER TABLE posts ADD COLUMN label_source TEXT DEFAULT NULL;
ALTER TABLE posts ADD COLUMN review_pending INTEGER DEFAULT 0;
ALTER TABLE posts ADD COLUMN appeal_status TEXT DEFAULT NULL;
ALTER TABLE posts ADD COLUMN appeal_pow_hash BLOB DEFAULT NULL;
ALTER TABLE posts ADD COLUMN duplicate_flag INTEGER DEFAULT 0;
ALTER TABLE posts ADD COLUMN rate_warning INTEGER DEFAULT 0;
```

### 8.2 Reputation Table

```sql
CREATE TABLE IF NOT EXISTS reputation (
    identity_pubkey BLOB PRIMARY KEY,
    positive_points REAL DEFAULT 0.0,
    negative_points REAL DEFAULT 0.0,
    connection_count INTEGER DEFAULT 0,
    age_days INTEGER DEFAULT 0,
    warming_active INTEGER DEFAULT 1,
    last_decay_at INTEGER DEFAULT 0,
    -- Reporter track record
    upheld_reports INTEGER DEFAULT 0,
    dismissed_reports INTEGER DEFAULT 0,
    reversed_reports INTEGER DEFAULT 0,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
```

### 8.3 Reports Table (Persistence)

```sql
CREATE TABLE IF NOT EXISTS reports (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    reporter_pubkey BLOB NOT NULL,
    content_hash BLOB NOT NULL,
    reason TEXT NOT NULL,
    description TEXT,
    timestamp INTEGER NOT NULL,
    -- Resolution tracking
    resolution TEXT DEFAULT NULL,  -- 'upheld', 'dismissed', 'reversed'
    resolved_at INTEGER DEFAULT NULL,
    coordination_flag INTEGER DEFAULT 0,
    UNIQUE(reporter_pubkey, content_hash)
);
```

### 8.4 Moderation Actions Table

```sql
CREATE TABLE IF NOT EXISTS moderation_actions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    content_hash BLOB NOT NULL,
    action_type TEXT NOT NULL,  -- 'tombstone', 'label', 'dismiss', 'appeal_upheld', 'appeal_reversed'
    reason TEXT NOT NULL,
    weighted_score REAL,
    reporter_count INTEGER,
    timestamp INTEGER NOT NULL,
    -- For quorum-voted actions
    quorum_votes TEXT DEFAULT NULL  -- JSON array of moderator votes
);
```

---

## 9. Monitoring and Metrics

The following metrics should be tracked to evaluate the health of the moderation system:

| Metric | What it measures | Alert threshold |
|--------|-----------------|-----------------|
| `reports_per_hour` | Volume of incoming reports | > 100/hour (possible coordinated campaign) |
| `tombstones_per_hour` | Volume of auto-tombstones | > 20/hour (possible mass abuse or mass false reporting) |
| `appeals_per_hour` | Volume of appeals | > 10/hour (users are unhappy with moderation) |
| `appeal_reversal_rate` | Fraction of appeals that succeed | > 30% (auto-tombstone thresholds may be too aggressive) |
| `reporter_accuracy_median` | Median reporter accuracy across all reporters | < 0.5 (report quality is poor) |
| `posts_dropped_rate_limit` | Posts dropped at gossip ingest due to rate limiting | Informational (measure spam pressure) |
| `posts_dropped_reputation` | Posts dropped at gossip ingest due to low reputation | Informational (measure Sybil pressure) |
| `posts_dropped_duplicate` | Posts dropped at gossip ingest due to SimHash match | Informational (measure copy-paste spam) |
| `coordination_signals` | Number of coordination signals detected | > 5/day (active brigading campaign) |

These metrics are node-local (each node tracks its own). They can be exposed via the JSON-RPC API for the desktop client to display in a "Node Health" dashboard.

---

## 10. What This Spec Deliberately Does NOT Do

| Omission | Reason |
|----------|--------|
| Break encryption for moderation | Non-negotiable. See `05_moderation_safety.md` Section 12. |
| Permanent bans | Unenforceable in a pseudonymous system. New identity = 30-second PoW + 7-day warming. Reputation is the lever. |
| Automated text classification (ML) | Requires training data, introduces bias, false-positive risk is too high for a privacy platform. Heuristics + community moderation is the approach. |
| Content fingerprinting beyond CSAM | Scope creep. CSAM is the narrow, universally-condemned exception. See `05_moderation_safety.md` Section 12. |
| Single-node moderation authority | Every moderation action above labeling requires quorum consensus. No single node, no single moderator, no single government can unilaterally censor content. |
| Reporting to law enforcement | The platform does not have user identity information to report. CSAM hash matches are reported to NCMEC with content hashes only (no user metadata). See `05_moderation_safety.md` Section 11. |

---

## 11. Testing Strategy

### 11.1 Unit Tests (Per Module)

Each module listed in the implementation plan includes specific test cases. All tests follow the pattern:
1. Set up the defense (rate limiter, reputation store, content filter, etc.)
2. Simulate the attack (rapid posts, low-rep author, blocklisted content, etc.)
3. Assert the defense works (post rejected, rate limited, flagged, etc.)
4. Assert false positives are handled (normal content passes, established users get leniency, etc.)

### 11.2 Integration Tests

**File:** `tests/integration/moderation_pipeline.rs`

| Test | Description |
|------|-------------|
| `test_full_gossip_pipeline_clean_post` | Clean post from established user passes all checks and is stored |
| `test_full_gossip_pipeline_spam_flood` | 50 identical posts from same author; first 10 accepted, rest dropped |
| `test_full_gossip_pipeline_toxic_author` | Post from rep < -50 author dropped at step 5 |
| `test_full_gossip_pipeline_warming_media` | Media post from warming identity dropped at step 5 |
| `test_full_gossip_pipeline_blocklisted` | Post with blocklisted content hash dropped at step 6 |
| `test_full_local_pipeline_rate_limited` | Local user creating posts too fast gets rate limit error |
| `test_full_local_pipeline_warming_restrictions` | New identity restricted to text-only, 1/hour |
| `test_report_to_tombstone_flow` | File enough qualified reports to trigger auto-tombstone |
| `test_report_weaponization_resisted` | 10 reports from warming accounts do not trigger tombstone |
| `test_appeal_reversal_penalizes_reporters` | Successful appeal reduces reporters' track records |
| `test_coordination_detection_doubles_threshold` | Coordinated reports detected, threshold doubled, tombstone not triggered |
| `test_brigading_protection` | Author targeted by multiple reports gets tripled threshold |

### 11.3 Adversarial Tests

**File:** `tests/integration/adversarial_moderation.rs`

These tests simulate realistic attack scenarios:

| Test | Scenario |
|------|----------|
| `test_sybil_spam_attack` | Create 50 identities (all warming), each posts 1/hour. Verify: all rate-limited correctly, none can post media, spam is deduplicated. |
| `test_false_report_brigade` | 20 accounts report the same political post as "Hate Speech". Verify: coordination detected, threshold doubled, content gets label not tombstone. |
| `test_csam_single_report` | Single report of CSAM. Verify: immediate tombstone, content hash added to blocklist. |
| `test_report_then_harass` | Account A reports Account B, then sends threatening DMs. Verify: DM rate limiting protects B, report is still valid. |
| `test_moderator_with_conflict` | Moderator has mutual connection with reporter. Verify: moderator is recused from the vote. |

---

*This document is part of the Ephemera Architecture series. See [ARCHITECTURE.md](./ARCHITECTURE.md) for the master document.*
*Companion document: [05_moderation_safety.md](./05_moderation_safety.md) for design philosophy and data structures.*
