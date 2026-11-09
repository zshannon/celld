// Copyright 2026 Deno Land Inc. Apache-2.0 license.

//! The output gate: one choke point every egress passes through.
//!
//! # The rule
//!
//! Nothing that reveals a cell's state may leave this process while that cell
//! has a confirmation-required write it cannot prove durable. A client told about a write that a
//! crash then discards has been told something false, and no later message can
//! retract a side effect that has already left.
//!
//! # The implementation contract
//!
//! [`Channel`] enumerates every route that can reveal a cell's state. The shell
//! holds each effect and sends an [`Event::OutputAt`] with its route and the
//! release-time monotonic instant. This module
//! applies one durability rule to every route and returns the same route in an
//! [`Effect::Release`].
//!
//! Each part of that contract has one implementation counterpart:
//!
//! | contract | implementation |
//! | --- | --- |
//! | enumerate every output route | the [`Channel`] enum |
//! | apply one rule to every route | the `channel` argument to [`State::output`] |
//! | release only revealable state | the output's barrier, or the barrier it trails, has settled |
//! | let the held effect leave | [`Effect::Release`] with `Ok` |
//!
//! All external channels share one rule. [`Channel::Sync`] is an explicit
//! durability waiter, not an external output: its barrier waits for all writes
//! but does not recruit unrelated outputs behind unconfirmed writes.
//!
//! # Why one choke point
//!
//! Earlier implementations split this rule between a core reader gate, a shell
//! WebSocket queue, and an in-handler position check. Each mechanism had a
//! different view of the outstanding writes, so one channel could miss a write
//! that another mechanism knew about. One core barrier map gives every channel
//! the same view.
//!
//! # The second way a barrier opens
//!
//! Every channel passes through [`State::output`]. Not every barrier is opened
//! by one: `alarm_finished` opens its own when a firing's consuming commit is
//! unproven, owned by [`GateOwner::Alarm`] rather than by an output. An alarm
//! is not a channel -- it settles by replay into the core, not by
//! release to the shell, so a [`Channel`] variant could not carry it -- but its
//! barrier lives in the same per-cell map, which is what matters here: a
//! read-only output trails the newest barrier on its cell without asking who
//! opened it, so it trails an alarm's unproven write exactly as it trails a
//! request's.
//!
//! An earlier alarm path proved its write inside [`Effect::FireAlarm`] without
//! registering a core barrier. A read-only output could therefore miss that
//! unproven write. The alarm now opens a barrier in the same map, so every
//! reader observes it.
//!
//! # A streamed body is more than one output
//!
//! The model treats a response as atomic, and for a complete response it is:
//! one release covers every byte. A response whose body streams is not atomic.
//! The head is released while nothing is pending, and the producer then keeps
//! running; a chunk it makes later can read a commit another event made after
//! that release. So the shell takes one ticket per chunk on
//! [`Channel::Response`], not one ticket for the response. The producer is a
//! single pump that awaits each chunk's ticket before it asks for the next
//! chunk, which is what keeps a held chunk from overtaking a released one.

use crate::*;

impl Barrier {
    fn blocks_output(&self) -> bool {
        !matches!(
            self.owner,
            GateOwner::Output(Held {
                channel: Channel::Sync,
                ..
            })
        )
    }
}

impl State {
    /// **The output gate. Every channel passes through here.**
    ///
    /// The implementation contract is meant to be checked line by line:
    ///
    /// - `channel` names the route. External outputs share one rule; explicit
    ///   sync waiters prove their position without holding other outputs.
    /// - no barrier means that no unproven write remains for this output to
    ///   reveal, so the core emits [`Effect::Release`] with `Ok`.
    ///
    /// A write opens its own barrier and waits for a proof that covers its
    /// position. A read-only output that observed a committed write no
    /// verified proof covers yet waits for that proof itself: the handler
    /// that wrote takes its ticket only when it answers, so a reader that
    /// answers before that finds no barrier for what it read. Otherwise a
    /// read-only output joins the newest barrier open on its cell, because a
    /// reader can start after a write commits and before its proof lands.
    /// Comparing the reader's own start and end positions therefore decides
    /// nothing.
    pub(crate) fn output(
        &mut self,
        request: RequestId,
        channel: Channel,
        position: Option<u64>,
        observed: Option<u64>,
        epoch: Option<Epoch>,
        effects: &mut Vec<Effect>,
    ) {
        let held = Held { request, channel };
        // Authority is checked here, not only at request start: a handler can
        // start under a live node lease and reach its output after that lease
        // expired. Releasing then would reveal work a successor owner can
        // already have replaced.
        if !self.node_authoritative() {
            effects.push(Effect::Release {
                request,
                channel,
                result: Err(RequestError::NodeFenced),
            });
            return;
        }
        // A ticket sampled before its request pinned the cell names the epoch
        // it sampled at. A reset between the sample and the pin discarded that
        // epoch's unproven writes, and the pin then activated the next epoch:
        // a proof there covers nothing the ticket asks about, so the ticket
        // fails rather than acknowledge a discarded write. A request whose
        // cell is not resident at all fails here too, rather than by the
        // branch below: the write branch refuses that case itself, but the
        // read-only branch releases an active request whatever its cell's
        // phase, and a sampled epoch is a claim this check must settle
        // instead of leaving it to the reset's request cleanup.
        if let Some(sampled) = epoch {
            if self
                .resident_cell(request)
                .is_none_or(|(_, current)| current != sampled)
            {
                effects.push(Effect::Release {
                    request,
                    channel,
                    result: Err(RequestError::DurabilityUnproven),
                });
                return;
            }
        }
        match (position, observed) {
            (Some(position), _) => self.open_write_barrier(held, position, effects),
            (None, Some(observed)) => match self.resident_cell(request) {
                Some((cell, epoch)) if self.proven_covers(&cell, epoch, observed) => {
                    self.trail_open_barrier(held, effects)
                }
                Some((cell, epoch)) => self.hold_observed(held, cell, epoch, observed, effects),
                // The ticket names a commit no proof covers, on a cell that is
                // no longer resident to prove it. Fail closed, as a write in
                // that state does, rather than release what a read-only
                // ticket on a cell with nothing to prove would release.
                None => effects.push(Effect::Release {
                    request,
                    channel,
                    result: Err(RequestError::DurabilityUnproven),
                }),
            },
            (None, None) => self.trail_open_barrier(held, effects),
        }
    }

    /// Whether a verified proof of this residency, at `epoch`, already covers
    /// `observed`.
    fn proven_covers(&self, cell: &CellId, epoch: Epoch, observed: u64) -> bool {
        self.cells
            .get(cell)
            .and_then(|record| record.proven_position)
            .is_some_and(|(proven_at, proven)| proven_at == epoch && proven >= observed)
    }

    /// Hold a read-only output that observed a committed write no verified
    /// proof covers yet. It joins a barrier on its cell whose position covers
    /// what it observed, and opens one at that position otherwise: the proof
    /// is then the reader's own, and the writer's later ticket opens a barrier
    /// a proof already satisfies.
    fn hold_observed(
        &mut self,
        held: Held,
        cell: CellId,
        epoch: Epoch,
        observed: u64,
        effects: &mut Vec<Effect>,
    ) {
        if let Some((_, barrier)) = self.barriers.iter_mut().rev().find(|(_, barrier)| {
            barrier.cell == cell
                && barrier.epoch == epoch
                && barrier.position >= observed
                && barrier.blocks_output()
        }) {
            barrier.followers.push(held);
            return;
        }
        self.open_write_barrier(held, observed, effects);
    }

    /// Record a verified proof, so a later read-only output that observed no
    /// more than `position` at this epoch releases without a proof of its own.
    fn note_proven(&mut self, cell: &CellId, epoch: Epoch, position: u64) {
        let Some(record) = self.cells.get_mut(cell) else {
            return;
        };
        record.proven_position = Some(match record.proven_position {
            Some((proven_at, proven)) if proven_at == epoch => (epoch, proven.max(position)),
            _ => (epoch, position),
        });
    }

    /// Open the output gate for a local write: hold its output until the
    /// cell's committed `position` is proven replicated. The request must still
    /// be a live local activity on its cell, resident at the epoch that
    /// committed the write; otherwise durability cannot be proven for it and
    /// the output fails rather than falsely acknowledging the write.
    fn open_write_barrier(&mut self, held: Held, position: u64, effects: &mut Vec<Effect>) {
        let request = held.request;
        let Some((cell, epoch)) = self.resident_cell(request) else {
            effects.push(Effect::Release {
                request,
                channel: held.channel,
                result: Err(RequestError::DurabilityUnproven),
            });
            return;
        };
        if self.proven_covers(&cell, epoch, position) {
            effects.push(Effect::Release {
                request,
                channel: held.channel,
                result: Ok(()),
            });
            return;
        }
        let op = self.op();
        self.barriers.insert(
            op,
            Barrier {
                owner: GateOwner::Output(held),
                cell: cell.clone(),
                epoch,
                position,
                followers: Vec::new(),
            },
        );
        effects.push(Effect::AwaitDurable {
            op,
            cell,
            epoch,
            position,
        });
    }

    /// The cell a live local request runs against, and the epoch it is
    /// resident at. `None` when the request is not active or its cell is not
    /// resident.
    fn resident_cell(&self, request: RequestId) -> Option<(CellId, Epoch)> {
        self.active_requests.get(&request).and_then(|id| {
            match self.cells.get(id).map(|cell| &cell.phase) {
                Some(Phase::Resident { epoch }) => Some((id.clone(), *epoch)),
                _ => None,
            }
        })
    }

    /// Hold a read-only output behind the newest barrier open on its cell.
    ///
    /// Residency is not required here, unlike a write: a read on a cell that
    /// has left `Resident` has no barrier to find and no write of its own to
    /// prove, so it releases rather than fails.
    fn trail_open_barrier(&mut self, held: Held, effects: &mut Vec<Effect>) {
        let Some(cell) = self.active_requests.get(&held.request).cloned() else {
            effects.push(Effect::Release {
                request: held.request,
                channel: held.channel,
                result: Err(RequestError::DurabilityUnproven),
            });
            return;
        };
        if let Some((_, barrier)) = self
            .barriers
            .iter_mut()
            .rev()
            .find(|(_, barrier)| barrier.cell == cell && barrier.blocks_output())
        {
            barrier.followers.push(held);
        } else {
            effects.push(Effect::Release {
                request: held.request,
                channel: held.channel,
                result: Ok(()),
            });
        }
    }

    /// A gated write's durability proof completed. Acknowledge the write only
    /// when the replica proved a position that *covers* it — a shorter proof
    /// (a lagging or lying replicator) fails it rather than acknowledging a
    /// write the node cannot actually restore. Any error fails it. A completion
    /// for a gate already drained (fence or deadline) is ignored — the
    /// versioned-op discipline used throughout the core.
    pub(crate) fn durable_reached(
        &mut self,
        op: OpId,
        result: Result<u64, Failure>,
        source: ProofSource,
        effects: &mut Vec<Effect>,
    ) {
        let Some(gate) = self.barriers.get(&op) else {
            return;
        };
        let proven = matches!(result, Ok(durable) if durable >= gate.position);
        // A bucket proof reveals nothing until an ownership read confirms the
        // record still names this node at this epoch: "durable in `e<epoch>/`" is not
        // durable if the prefix was orphaned, and the bucket cannot refuse a
        // stale writer. A fleet proof needs no read — the ensemble
        // arbitrated it: a takeover seals a member before restoring, so a
        // stale owner's ack-all fails closed.
        if proven && source == ProofSource::Bucket {
            let (cell, epoch) = (gate.cell.clone(), gate.epoch);
            effects.push(Effect::VerifyOwnership { op, cell, epoch });
            return;
        }
        self.settle_gate(op, proven, effects);
    }

    /// Ownership verification for a bucket-proof gate. A record that no longer
    /// names this node at this epoch fails the write exactly like an unproven
    /// durability result: refuse and reset, never acknowledge into an
    /// orphaned lineage.
    pub(crate) fn ownership_verified(
        &mut self,
        op: OpId,
        result: Result<(), Failure>,
        effects: &mut Vec<Effect>,
    ) {
        if !self.barriers.contains_key(&op) {
            return;
        }
        self.settle_gate(op, result.is_ok(), effects);
    }

    /// Release one withheld output, unpinning it first so the cleanup that
    /// ends -- shedding, eviction -- is queued ahead of the release.
    pub(crate) fn release_held(
        &mut self,
        held: Held,
        result: Result<(), RequestError>,
        effects: &mut Vec<Effect>,
    ) {
        if self.gate_pinned.remove(&held.request) {
            self.activity_finished(held.request, effects);
        }
        effects.push(Effect::Release {
            request: held.request,
            channel: held.channel,
            result,
        });
    }

    fn settle_gate(&mut self, op: OpId, proven: bool, effects: &mut Vec<Effect>) {
        let Some(gate) = self.barriers.remove(&op) else {
            return;
        };
        let result = if proven {
            self.note_proven(&gate.cell, gate.epoch, gate.position);
            Ok(())
        } else {
            Err(RequestError::DurabilityUnproven)
        };
        match gate.owner {
            // Unpin before releasing: the cleanup this ends -- shedding,
            // eviction -- is queued ahead of the response, so a caller that
            // sees its write acknowledged sees the residency it released too.
            // `activity_finished` re-checks the barrier map, so a request
            // holding a second gated write simply re-pins itself here.
            GateOwner::Output(held) => self.release_held(held, result, effects),
            // The alarm settles only now. A proven commit replays the
            // observation the handler made, which routes through
            // `alarm_observed` and orders the consume-side wake-entry delete
            // -- after the proof, by construction. An unproven one takes the
            // re-arm branch a failed handler takes, so the entry stays
            // discoverable and at-least-once holds. The replay carries no
            // position, so it settles rather than opening a second barrier.
            GateOwner::Alarm {
                alarm,
                snapshot,
                covered,
            } => {
                let outcome = if proven {
                    Ok((snapshot, covered, None))
                } else {
                    Err(Failure::Ambiguous)
                };
                let (now_ms, now_mono_ms) = (self.now_ms, self.now_mono_ms);
                self.alarm_finished(
                    alarm,
                    (gate.cell.clone(), gate.epoch),
                    now_ms,
                    now_mono_ms,
                    outcome,
                    effects,
                );
            }
        }
        // A follower is always an output: an alarm opens a barrier, it never
        // trails one.
        for held in gate.followers {
            self.release_held(held, result, effects);
        }
        if !proven {
            let (id, epoch) = (gate.cell.clone(), gate.epoch);
            self.reset_cell(&id, epoch, effects);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resident() -> State {
        let mut state = State::new(
            "test",
            Config {
                max_resident: 1,
                max_activations: 1,
                max_evictions: 1,
                max_releases: 1,
                max_outbound_websockets: 1,
                ownership_on_evict: OwnershipOnEvict::Release,
                require_node_lease: false,
                peer_protocol: 1,
                operation_deadline_ms: None,
                owner_log_recovery_backoff_ms: 0,
                owner_log_recovery_attempts: 1,
                alarm_resident_ms: 0,
                idle_evict_ms: None,
                pressure: Default::default(),
            },
        );
        state.cells.insert(
            "cell".into(),
            Cell {
                phase: Phase::Resident { epoch: 1 },
                ..Default::default()
            },
        );
        state.occupied = 1;
        for request in 1..=3 {
            state.activate_request(request, "cell".into());
        }
        state
    }

    fn output(
        state: &mut State,
        request: u64,
        channel: Channel,
        position: Option<u64>,
        observed: Option<u64>,
    ) -> Vec<Effect> {
        on_event(
            state,
            Event::Output {
                request,
                channel,
                position,
                observed,
                epoch: Some(1),
            },
        )
    }

    fn waiting(effects: &[Effect], expected: u64) -> OpId {
        let [Effect::AwaitDurable { op, position, .. }] = effects else {
            panic!("expected durability wait, got {effects:?}")
        };
        assert_eq!(*position, expected);
        *op
    }

    fn released(effects: &[Effect], expected: u64) -> bool {
        effects.iter().any(|effect| matches!(effect, Effect::Release { request, result: Ok(()), .. } if *request == expected))
    }

    fn prove(state: &mut State, op: OpId, position: u64) -> Vec<Effect> {
        on_event(
            state,
            Event::DurableReached {
                op,
                result: Ok(position),
                source: ProofSource::Fleet,
            },
        )
    }

    #[test]
    fn sync_waits_for_proof_without_holding_any_network_channel() {
        let mut state = resident();
        let op = waiting(&output(&mut state, 1, Channel::Sync, Some(20), None), 20);
        for channel in [
            Channel::Response,
            Channel::Fetch,
            Channel::WsHibernatable,
            Channel::WsSelf,
            Channel::Service,
            Channel::CellRpc,
            Channel::R2,
            Channel::Tcp,
            Channel::Queue,
        ] {
            let effects = output(&mut state, 2, channel, None, None);
            assert!(released(&effects, 2), "{channel:?}: {effects:?}");
            assert!(!released(&effects, 1));
        }
        assert!(released(&prove(&mut state, op, 20), 1));
    }

    #[test]
    fn confirmed_reader_does_not_join_a_later_sync_wait() {
        let mut state = resident();
        let sync = waiting(&output(&mut state, 1, Channel::Sync, Some(20), None), 20);
        let reader = waiting(
            &output(&mut state, 2, Channel::Response, None, Some(10)),
            10,
        );
        let effects = prove(&mut state, reader, 10);
        assert!(released(&effects, 2));
        assert!(!released(&effects, 1));
        assert!(released(&prove(&mut state, sync, 20), 1));
    }

    #[test]
    fn ordinary_write_still_holds_readers_while_sync_is_pending() {
        let mut state = resident();
        let write = waiting(
            &output(&mut state, 1, Channel::Response, Some(10), None),
            10,
        );
        let sync = waiting(&output(&mut state, 2, Channel::Sync, Some(20), None), 20);
        assert!(output(&mut state, 3, Channel::Fetch, None, None).is_empty());
        let effects = prove(&mut state, write, 10);
        assert!(released(&effects, 1));
        assert!(released(&effects, 3));
        assert!(!released(&effects, 2));
        assert!(released(&prove(&mut state, sync, 20), 2));
    }

    #[test]
    fn already_proven_write_does_not_wait_for_another_capture() {
        let mut state = resident();
        let op = waiting(
            &output(&mut state, 1, Channel::Response, Some(10), None),
            10,
        );
        prove(&mut state, op, 10);
        assert!(released(
            &output(&mut state, 2, Channel::Fetch, Some(10), None),
            2
        ));
    }

    #[test]
    fn failed_sync_rejects_instead_of_claiming_durability() {
        let mut state = resident();
        let op = waiting(&output(&mut state, 1, Channel::Sync, Some(20), None), 20);
        let effects = on_event(
            &mut state,
            Event::DurableReached {
                op,
                result: Err(Failure::Definite),
                source: ProofSource::Fleet,
            },
        );
        assert!(effects.iter().any(|effect| matches!(
            effect,
            Effect::Release {
                request: 1,
                result: Err(RequestError::DurabilityUnproven),
                ..
            }
        )));
    }
}
