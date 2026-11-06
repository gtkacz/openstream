# Resilient Room Tickets Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A ticket minted while others are in the room keeps working after its minter leaves, and a join whose first attempt fails retries before giving up.

**Architecture:** `Room::ticket()` mints the topic plus self and every member whose address the Watcher cached from a live connection — self first, most recently seen first. No wire-format change: `RoomTicket.bootstrap` is already a list, and old versions treat the extra entries as extra bootstrap candidates. `gossip::join` wraps subscribe-and-wait in a bounded retry loop, dropping the topic handle between attempts so each attempt re-dials the bootstrap list (iroh-gossip re-issues `ProtoCommand::Join` on every subscribe, so a re-subscribe genuinely re-attempts dials).

**Tech Stack:** Rust, iroh 1.1, iroh-gossip 0.101, tokio.

**Spec:** Layers 1+2 of the stale-ticket design agreed with the user in session on 2026-11-05. Layer 3 (a topic-keyed directory service) is explicitly out of scope.

## Global Constraints

- No wire-format change: `crates/proto/src/ticket.rs` stays untouched; old and new tickets are mutually parseable.
- `Room::ticket()` stays synchronous. Addresses come from a cache the Watcher fills on each successful member connection; `rejoin_ticket()` keeps its current shape and stays async.
- No cap on the bootstrap list (deliberate non-goal: rooms are small; revisit if tickets get unwieldy).
- `RoomError::JoinTimeout` and its user-facing message stay; retries happen before it.
- Tests run with `RelaySetting::Disabled`: every bootstrap address a test dials must be real. DNS/node-id-only resolution is unavailable in tests.
- The empty-bootstrap (creator) path must never enter the retry loop; a room of one joins immediately.

## Review Focus

- A ticket minted before anyone else joined still names only its creator and dies with them — expected; the fix covers tickets minted after the second member joins. Pinned by Task 1's test, which mints only after mutual presence.
- Cached addresses outlive their connections: a joiner may dial a stale address first. It must fail fast and reach the next candidate, not hang. Pinned by Task 1's test: the stale ticket carries the departed creator's dead address.
- Expired members must stop being named even though their cached addresses remain. Pinned by Task 1's test: after the creator expires at bob, bob's ticket no longer names alice.
- Re-subscribing a topic the actor still holds (its quit is processed asynchronously) must re-dial, not wedge the swarm. Pinned by Task 2's unit test, which subscribes repeatedly on one actor.
- Exhausted retries must still surface `RoomError::JoinTimeout` in bounded time. Pinned by `two_rooms.rs::a_bad_ticket_times_out_instead_of_hanging`, which keeps its sub-5s assertion with retries enabled.

---

### Task 1: Tickets carry the room

**Files:**
- Modify: `crates/room/src/watcher.rs` (cache in `Inner`, write in `client_for`, accessor)
- Modify: `crates/room/src/room.rs:287-290` (`ticket()`)
- Test: `crates/room/tests/hub_leaves.rs`

**Interfaces:**
- Consumes: existing `Watcher::client_for` (the single funnel for member connections), `Membership::members()` (`Member { id, last_seen, .. }`), `endpoint.remote_info(id).await -> Option<RemoteInfo>` as used in `rejoin_ticket`.
- Produces: `Watcher::member_addr(&self, id: &PublicKey) -> Option<EndpointAddr>` (`pub(crate)`); `Room::ticket()` — same signature, now returns topic + [self addr] + cached member addresses, most recently seen first.

- [ ] **Step 1: Write the failing test** in `crates/room/tests/hub_leaves.rs`:

```rust
#[tokio::test]
async fn a_ticket_minted_after_others_join_survives_the_creator_leaving() {
    let a = Room::create(config("alice")).await.unwrap();
    let b = Room::join(config("bob"), a.ticket()).await.unwrap();
    wait_until("mutual presence", Duration::from_secs(10), || {
        a.snapshot().members.len() == 1 && b.snapshot().members.len() == 1
    })
    .await;

    let a_id = a.id();
    let ticket = a.ticket();
    assert_eq!(ticket.bootstrap[0].id, a_id, "the minting member is the first bootstrap");
    assert!(
        ticket.bootstrap.iter().any(|addr| addr.id == b.id()),
        "the ticket names the other members"
    );

    let topic = ticket.topic;
    a.leave().await;
    let c = Room::join(config("carol"), ticket).await.unwrap();
    wait_until("carol joins through the stale ticket", Duration::from_secs(10), || {
        b.snapshot().members.iter().any(|m| m.nickname == "carol")
            && c.snapshot().members.iter().any(|m| m.nickname == "bob")
    })
    .await;
    assert_eq!(c.ticket().topic, topic, "the room is the same room");

    wait_until("alice expires at bob", Duration::from_secs(10), || {
        !b.snapshot().members.iter().any(|m| m.id == a_id)
    })
    .await;
    assert!(
        !b.ticket().bootstrap.iter().any(|addr| addr.id == a_id),
        "an expired member stops being named"
    );

    c.leave().await;
    b.leave().await;
}
```

- [ ] **Step 2: Run it and verify the failure**

Run: `cargo test -p brp_room --test hub_leaves a_ticket_minted_after_others_join_survives_the_creator_leaving`
Expected: FAIL at "the ticket names the other members" (today `ticket()` names only self).

- [ ] **Step 3: Implement**

In `crates/room/src/watcher.rs`:
- `Inner` gains `addrs: HashMap<PublicKey, EndpointAddr>` — the last address known for each member, from their most recent connection. No invalidation: `ticket()` filters by current membership, and a stale address only costs a joiner one failed dial.
- In `client_for`, after `MediaClient::connect` succeeds, record the member's current addresses, mirroring `rejoin_ticket`'s conversion:

```rust
if let Some(info) = self.endpoint.remote_info(publisher).await {
    let addr = EndpointAddr::from_parts(publisher, info.into_addrs().map(|a| a.into_addr()));
    lock(&self.inner).addrs.insert(publisher, addr);
}
```

- Accessor, next to `path_rtt`:

```rust
/// The last address a member was reachable at, from their most recent connection.
pub(crate) fn member_addr(&self, id: &PublicKey) -> Option<EndpointAddr> {
    lock(&self.inner).addrs.get(id).cloned()
}
```

In `crates/room/src/room.rs`, replace `ticket()`'s body and doc comment:

```rust
/// A ticket naming this participant first and every member whose address we hold, most
/// recently seen first. It keeps working while any named member stays online — including
/// after this participant leaves.
pub fn ticket(&self) -> RoomTicket {
    let mut bootstrap = vec![self.endpoint.addr()];
    let mut others: Vec<_> = lock(&self.membership)
        .members()
        .filter(|m| m.id != self.me)
        .filter_map(|m| self.watcher.member_addr(&m.id).map(|addr| (m.last_seen, addr)))
        .collect();
    others.sort_by(|a, b| b.0.cmp(&a.0));
    bootstrap.extend(others.into_iter().map(|(_, addr)| addr));
    RoomTicket::new(self.topic, bootstrap)
}
```

- [ ] **Step 4: Run the test and the neighbours**

Run: `cargo test -p brp_room --test hub_leaves --test two_rooms --test rejoin`
Expected: PASS — the new test, plus existing flows (`joined_pair` mints tickets before membership fills and must keep working).

- [ ] **Step 5: Commit**

```bash
git add crates/room/src/watcher.rs crates/room/src/room.rs crates/room/tests/hub_leaves.rs
git commit -m "feat(room): tickets name every member we hold an address for"
```

### Task 2: Bounded join retries

**Files:**
- Modify: `crates/proto/src/constants.rs` (add `JOIN_ATTEMPTS`)
- Modify: `crates/room/src/room.rs` (`RoomTimings.join_attempts`, pass to `gossip::join`)
- Modify: `crates/room/src/gossip.rs:20-38` (retry loop) and a new `#[cfg(test)] mod tests` there
- Modify: test config literals at `crates/room/tests/{hard_failure,two_rooms,rejoin,hub_leaves}.rs` and `crates/app/src/room_view.rs:356` (each gains `join_attempts`)

**Interfaces:**
- Consumes: `gossip.subscribe(topic, bootstrap)` returns instantly; `GossipTopic::joined()` hangs while no neighbor answers; dropping the `GossipTopic` makes the actor leave the topic; a fresh `subscribe` re-dials the bootstrap list.
- Produces: `gossip::join(gossip, topic, bootstrap, timeout: Duration, attempts: u32)`; `RoomTimings { join_attempts: u32, .. }`; `brp_proto::constants::JOIN_ATTEMPTS: u32 = 3`.

- [ ] **Step 1: Write the failing unit test** — new `#[cfg(test)] mod tests` at the end of `crates/room/src/gossip.rs` (check `crates/room/Cargo.toml` dev-dependencies provide `tokio` with `macros` and `rt`; add if missing):

```rust
#[cfg(test)]
mod tests {
    use std::time::Duration;

    use brp_net::RelaySetting;
    use iroh::protocol::Router;
    use iroh::{EndpointAddr, SecretKey};
    use iroh_gossip::net::Gossip;
    use iroh_gossip::proto::TopicId;

    use super::*;

    /// A bound endpoint whose router answers gossip, like a room's endpoint before its
    /// gossip::join has subscribed. `known` seeds the address lookup, the same way a
    /// ticket's bootstrap list does.
    async fn peer(known: Vec<EndpointAddr>) -> (iroh::Endpoint, Gossip, Router) {
        let endpoint =
            brp_net::bind_endpoint(SecretKey::generate(), RelaySetting::Disabled, known)
                .await
                .unwrap();
        let gossip = Gossip::builder().spawn(endpoint.clone());
        let router = Router::builder(endpoint.clone())
            .accept(iroh_gossip::ALPN, gossip.clone())
            .spawn();
        (endpoint, gossip, router)
    }

    #[tokio::test]
    async fn a_join_retries_until_the_bootstrap_answers() {
        let (a_endpoint, a_gossip, _a_router) = peer(Vec::new()).await;
        // b can reach a only through the address seeded at bind time.
        let (b_endpoint, b_gossip, _b_router) = peer(vec![a_endpoint.addr()]).await;
        let topic = TopicId::from_bytes([7u8; 32]);
        let bootstrap = vec![a_endpoint.addr().id];

        let joined = tokio::spawn(async move {
            join(&b_gossip, topic, bootstrap, Duration::from_millis(300), 4).await
        });
        // Attempts one and two fail while a has not subscribed to the topic.
        tokio::time::sleep(Duration::from_millis(700)).await;
        let _a = join(&a_gossip, topic, Vec::new(), Duration::from_secs(5), 1)
            .await
            .unwrap();

        let (sender, _receiver) = joined.await.unwrap().unwrap();
        drop(sender);
    }
}
```

- [ ] **Step 2: Run it and verify the failure**

Run: `cargo test -p brp_room --lib gossip`
Expected: FAIL to compile — `join` takes no `attempts` parameter (the feature is missing).

- [ ] **Step 3: Implement**

`crates/proto/src/constants.rs`, next to `JOIN_TIMEOUT`:

```rust
/// How many times a joiner re-dials the ticket's bootstrap list before giving up.
pub const JOIN_ATTEMPTS: u32 = 3;
```

`crates/room/src/gossip.rs` — `join` gains `attempts: u32` and loops; the creator path stays outside the loop:

```rust
pub(crate) async fn join(
    gossip: &Gossip,
    topic: TopicId,
    bootstrap: Vec<EndpointId>,
    timeout: Duration,
    attempts: u32,
) -> Result<(GossipSender, GossipReceiver), RoomError> {
    // A creator has nobody to join; everyone else must reach a neighbour or the ticket is dead.
    if bootstrap.is_empty() {
        let topic = gossip
            .subscribe(topic, Vec::new())
            .await
            .map_err(|e| RoomError::Gossip(e.to_string()))?;
        return Ok(topic.split());
    }
    for _ in 0..attempts {
        let mut topic = gossip
            .subscribe(topic, bootstrap.clone())
            .await
            .map_err(|e| RoomError::Gossip(e.to_string()))?;
        match tokio::time::timeout(timeout, topic.joined()).await {
            Ok(Ok(())) => return Ok(topic.split()),
            Ok(Err(error)) => tracing::debug!(%error, "gossip join attempt failed"),
            Err(_) => tracing::debug!("gossip join attempt timed out"),
        }
        // Dropping the topic handle makes the actor leave the topic, so the next attempt
        // re-dials the bootstrap list from scratch.
    }
    Err(RoomError::JoinTimeout)
}
```

`crates/room/src/room.rs`: `RoomTimings` gains `pub join_attempts: u32`, `Default` sets it to `JOIN_ATTEMPTS` (import from `brp_proto::constants`), and the `gossip::join` call site passes `config.timings.join_attempts`. Update the five config literals with `join_attempts: 3`.

- [ ] **Step 4: Run the tests**

Run: `cargo test -p brp_room --lib gossip --test two_rooms --test hub_leaves`
Expected: PASS — the retry unit test; `a_bad_ticket_times_out_instead_of_hanging` still errors with `JoinTimeout` in under 5s (3 × 500ms of attempts).

- [ ] **Step 5: Commit**

```bash
git add crates/proto/src/constants.rs crates/room/src/gossip.rs crates/room/src/room.rs crates/room/tests/ crates/app/src/room_view.rs
git commit -m "feat(room): retry the gossip join before reporting a dead ticket"
```

### Task 3: Full verification

- [ ] `cargo test --workspace` — all pass, including the two new tests.
- [ ] `cargo clippy --all-targets` and `cargo fmt --check` — clean (the pre-existing `clippy::bool_assert_comparison` allowance in window.rs stays untouched).
- [ ] Read the full diff once against the Global Constraints; confirm `crates/proto/src/ticket.rs` is untouched.
