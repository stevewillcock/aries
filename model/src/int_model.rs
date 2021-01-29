mod explanation;

pub use explanation::*;

use crate::expressions::ExprHandle;
use crate::lang::{IntCst, VarRef};
use crate::{Label, WriterId};
use aries_backtrack::{Backtrack, BacktrackWith};
use aries_backtrack::{TrailLoc, Q};
use aries_collections::ref_store::{RefMap, RefVec};
use std::cmp::Ordering;
use std::collections::BinaryHeap;

#[derive(Clone)]
pub struct IntDomain {
    pub lb: IntCst,
    pub ub: IntCst,
}
impl IntDomain {
    pub fn new(lb: IntCst, ub: IntCst) -> IntDomain {
        IntDomain { lb, ub }
    }
}
#[derive(Copy, Clone, Debug)]
pub struct VarEvent {
    pub var: VarRef,
    pub ev: DomEvent,
}

#[derive(Copy, Clone, Debug)]
pub enum DomEvent {
    NewLB { prev: IntCst, new: IntCst },
    NewUB { prev: IntCst, new: IntCst },
}

#[derive(Copy, Clone, Debug)]
pub enum Cause {
    Decision,
    /// The event is due to an inference.
    /// A WriterID identifies the module that made the inference.
    /// 64 bits are available for the writer to store additional metadata of the inference made.
    /// These can for instance be used to indicate the particular constraint that caused the change.
    /// When asked to explain an inference, both fields are made available to the explainer.
    Inference(InferenceCause),
}
impl Cause {
    pub fn inference(writer: WriterId, payload: impl Into<u64>) -> Self {
        Cause::Inference(InferenceCause {
            writer,
            payload: payload.into(),
        })
    }
}

#[derive(Copy, Clone, Debug)]
pub struct InferenceCause {
    /// A WriterID identifies the module that made the inference.
    pub writer: WriterId,
    /// 64 bits are available for the writer to store additional metadata of the inference made.
    /// These can for instance be used to indicate the particular constraint that caused the change.
    /// When asked to explain an inference, both fields are made available to the explainer.
    pub payload: u64,
}

#[derive(Default, Clone)]
pub struct DiscreteModel {
    labels: RefVec<VarRef, Label>,
    pub(crate) domains: RefVec<VarRef, IntDomain>,
    pub(crate) trail: Q<(VarEvent, Cause)>,
    pub(crate) expr_binding: RefMap<ExprHandle, ILit>,
}

impl DiscreteModel {
    pub fn new() -> DiscreteModel {
        DiscreteModel {
            labels: Default::default(),
            domains: Default::default(),
            trail: Default::default(),
            expr_binding: Default::default(),
        }
    }

    pub fn new_discrete_var<L: Into<Label>>(&mut self, lb: IntCst, ub: IntCst, label: L) -> VarRef {
        let id1 = self.labels.push(label.into());
        let id2 = self.domains.push(IntDomain::new(lb, ub));
        debug_assert_eq!(id1, id2);
        id1
    }

    pub fn variables(&self) -> impl Iterator<Item = VarRef> {
        self.labels.keys()
    }

    pub fn label(&self, var: impl Into<VarRef>) -> Option<&str> {
        self.labels[var.into()].get()
    }

    pub fn domain_of(&self, var: impl Into<VarRef>) -> &IntDomain {
        &self.domains[var.into()]
    }

    fn dom_mut(&mut self, var: impl Into<VarRef>) -> &mut IntDomain {
        &mut self.domains[var.into()]
    }

    // todo: should return a result
    pub fn set_lb(&mut self, var: impl Into<VarRef>, lb: IntCst, cause: Cause) {
        let var = var.into();
        let dom = self.dom_mut(var);
        let prev = dom.lb;
        if prev < lb {
            dom.lb = lb;
            let event = VarEvent {
                var,
                ev: DomEvent::NewLB { prev, new: lb },
            };
            self.trail.push((event, cause));
        }
    }

    pub fn set_ub(&mut self, var: impl Into<VarRef>, ub: IntCst, cause: Cause) {
        let var = var.into();
        let dom = self.dom_mut(var);
        let prev = dom.ub;
        if prev > ub {
            dom.ub = ub;
            let event = VarEvent {
                var,
                ev: DomEvent::NewUB { prev, new: ub },
            };
            self.trail.push((event, cause));
        }
    }

    // ================== Explanation ==============

    pub fn explain_empty_domain(&mut self, var: VarRef, explainer: &impl Explainer) -> Vec<ILit> {
        self.trail.print();

        #[derive(Copy, Clone, Debug)]
        struct InQueueLit {
            cause: TrailLoc,
            lit: ILit,
        };
        impl PartialEq for InQueueLit {
            fn eq(&self, other: &Self) -> bool {
                self.cause == other.cause
            }
        }
        impl Eq for InQueueLit {}
        impl Ord for InQueueLit {
            fn cmp(&self, other: &Self) -> Ordering {
                self.cause.cmp(&other.cause)
            }
        }
        impl PartialOrd for InQueueLit {
            fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
                Some(self.cmp(other))
            }
        }

        // literals falsified at the current decision level, we need to proceed until there is a single one left (1UIP)
        let mut queue: BinaryHeap<InQueueLit> = BinaryHeap::new();
        // literals that are beyond the current decision level and will be part of the final clause
        let mut result: Vec<ILit> = Vec::new();

        // working memory to let the explainer push its literals (without allocating memory)
        let mut explanation = Explanation::new();
        let IntDomain { lb, ub } = self.domain_of(var);

        // (lb <= X && X <= ub) => false
        // add (lb <= X) and (X <= ub) to explanation
        // TODO: this should be based on the initial domain
        if *lb > IntCst::MIN {
            explanation.push(ILit::GT(var, lb - 1));
        }
        if *ub < IntCst::MAX {
            explanation.push(ILit::LEQ(var, *ub));
        }

        let decision_level = self.trail.current_decision_level();

        loop {
            println!("before processing explanation: {:?}", queue.iter().collect::<Vec<_>>());
            for l in explanation.lits.drain(..) {
                assert!(self.entails(&l));
                // find the location of the event that made it true
                // if there is no such event, it means that the literal is implied in the initial state and we can ignore it
                if let Some(loc) = self.implying_event(&l) {
                    if loc.decision_level == decision_level {
                        // at the current decision level, add to the queue
                        queue.push(InQueueLit { cause: loc, lit: l })
                    } else {
                        // implied before the current decision level, the negation of the literal will appear in the final clause (1UIP)
                        result.push(!l)
                    }
                }
            }
            println!("after processing explanation: {:?}", queue.iter().collect::<Vec<_>>());
            assert!(!queue.is_empty());

            // hot reached the first UIP yet
            // select latest falsified literal from queue
            let l = queue.pop().unwrap();
            println!("next: {:?}", l);
            assert!(l.cause.event_index < self.trail.num_events());
            assert!(self.entails(&l.lit));
            let mut cause = None;
            // backtrack until the latest falsifying event
            // this will undo some of the change but will keep us in the same decision level
            while l.cause.event_index < self.trail.num_events() {
                let x = self.trail.pop().unwrap();
                Self::undo_int_event(&mut self.domains, x.0);
                cause = Some(x);
            }
            let cause = cause.unwrap();
            assert!(l.lit.made_true_by(&cause.0));

            match cause.1 {
                Cause::Decision => {
                    assert!(queue.is_empty());
                    result.push(!l.lit);
                    return result;
                }
                Cause::Inference(cause) => {
                    // ask for a clause (l1 & l2 & ... & ln) => lit
                    explainer.explain(cause, l.lit, &self, &mut explanation);
                }
            }
        }
    }

    pub(crate) fn entails(&self, lit: &ILit) -> bool {
        match lit {
            ILit::LEQ(var, val) => self.domain_of(*var).ub <= *val,
            ILit::GT(var, val) => self.domain_of(*var).lb > *val,
        }
    }

    fn implying_event(&self, lit: &ILit) -> Option<TrailLoc> {
        debug_assert!(self.entails(lit));
        let not_lit = !*lit;
        self.falsifying_event(&not_lit)
    }

    fn falsifying_event(&self, lit: &ILit) -> Option<TrailLoc> {
        let ev = self
            .trail
            .last_event_matching(|(ev, _)| lit.made_false_by(ev), |_, _| true);
        ev.map(|x| x.loc)
    }

    // ============= UNDO ================

    fn undo_int_event(domains: &mut RefVec<VarRef, IntDomain>, ev: VarEvent) {
        let dom = &mut domains[ev.var];
        match ev.ev {
            DomEvent::NewLB { prev, new } => {
                debug_assert_eq!(dom.lb, new);
                dom.lb = prev;
            }
            DomEvent::NewUB { prev, new } => {
                debug_assert_eq!(dom.ub, new);
                dom.ub = prev;
            }
        }
    }

    // =============== BOOL ===============

    // ================ EXPR ===========

    pub fn interned_expr(&self, handle: ExprHandle) -> Option<ILit> {
        self.expr_binding.get(handle).copied()
    }

    pub fn intern_expr_with(&mut self, handle: ExprHandle, make_lit: impl FnOnce() -> ILit) -> ILit {
        match self.interned_expr(handle) {
            Some(lit) => lit,
            None => {
                let lit = make_lit();
                self.bind_expr(handle, lit);
                lit
            }
        }
    }

    fn bind_expr(&mut self, handle: ExprHandle, literal: ILit) {
        self.expr_binding.insert(handle, literal);
    }
}

impl Backtrack for DiscreteModel {
    fn save_state(&mut self) -> u32 {
        self.trail.save_state()
    }

    fn num_saved(&self) -> u32 {
        self.trail.num_saved()
    }

    fn restore_last(&mut self) {
        let int_domains = &mut self.domains;
        self.trail
            .restore_last_with(|(ev, _)| Self::undo_int_event(int_domains, ev));
    }

    fn restore(&mut self, saved_id: u32) {
        let int_domains = &mut self.domains;
        self.trail
            .restore_with(saved_id, |(ev, _)| Self::undo_int_event(int_domains, ev));
    }
}

#[cfg(test)]
mod tests {
    use crate::assignments::Assignment;
    use crate::int_model::explanation::{Explainer, Explanation, ILit};
    use crate::int_model::{Cause, DiscreteModel, InferenceCause};
    use crate::lang::{BVar, IVar};
    use crate::{Model, WriterId};
    use aries_backtrack::Backtrack;
    use std::collections::HashSet;

    #[test]
    fn test_explanation() {
        let mut model = Model::new();
        let a = model.new_bvar("a");
        let b = model.new_bvar("b");
        let n = model.new_ivar(0, 10, "a");

        // constraint 0: "a => (n <= 4)"
        // constraint 1: "b => (n >= 5)"

        let writer = WriterId::new(1);

        let cause_a = Cause::inference(writer, 0u64);
        let cause_b = Cause::inference(writer, 1u64);

        let propagate = |model: &mut Model| {
            if model.boolean_value_of(a) == Some(true) {
                model.discrete.set_ub(n, 4, cause_a);
            }
            if model.boolean_value_of(b) == Some(true) {
                model.discrete.set_lb(n, 5, cause_b);
            }
        };

        struct Expl {
            a: BVar,
            b: BVar,
            n: IVar,
        }
        impl Explainer for Expl {
            fn explain(
                &self,
                cause: InferenceCause,
                literal: ILit,
                _model: &DiscreteModel,
                explanation: &mut Explanation,
            ) {
                assert_eq!(cause.writer, WriterId::new(1));
                match cause.payload {
                    0 => {
                        assert_eq!(literal, ILit::leq(self.n, 4));
                        explanation.push(ILit::is_true(self.a));
                    }
                    1 => {
                        assert_eq!(literal, ILit::geq(self.n, 5));
                        explanation.push(ILit::is_true(self.b));
                    }
                    _ => panic!("unexpected payload"),
                }
            }
        }

        let network = Expl { a, b, n };

        propagate(&mut model);
        model.save_state();
        model.discrete.set_lb(a, 1, Cause::Decision);
        propagate(&mut model);
        assert_eq!(model.bounds(n), (0, 4));
        model.save_state();
        model.discrete.set_lb(b, 1, Cause::Decision);
        propagate(&mut model);
        assert_eq!(model.bounds(n), (5, 4));

        let mut clause = model.discrete.explain_empty_domain(n.into(), &network);
        let clause: HashSet<_> = clause.drain(..).collect();

        // we have three rules
        //  -  !(n <= 4) || !(n >= 5)   (conflict)
        //  -  !a || (n <= 4)           (clause a)
        //  -  !b || (n >= 5)           (clause b)
        // Explanation should perform resolution of the first and last rules for the literal (n >= 5):
        //   !(n <= 4) || !b
        //   !b || (n > 4)      (equivalent to previous)
        let mut expected = HashSet::new();
        expected.insert(ILit::is_false(b));
        expected.insert(ILit::gt(n, 4));
        assert_eq!(clause, expected);
    }
}
