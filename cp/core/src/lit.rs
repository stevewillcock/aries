use crate::*;
use core::convert::{From, Into};
use std::cmp::Ordering;

/// A literal `Lit` represents a lower or upper bound on a discrete variable
/// (i.e. an integer, boolean or symbolic variable).
///
/// For a boolean variable X:
///  - the bound `x > 0` represent the true literal (`X` takes the value `true`)
///  - the bound `x <= 0` represents the false literal (`X` takes the value `false`)
///
/// The struct is opaque as it is internal representation is optimized to allow more efficient usage.
/// To access individual fields the methods `variable()`, `relation()` and `value()` can be used.
/// The `unpack()` method extract all fields into a tuple.
///
/// ```
/// use aries_core::*;
/// use aries_core::state::IntDomains;
/// let mut state = IntDomains::new();
/// let x = state.new_var(0, 1);
/// let x_is_true: Lit = x.geq(1);
/// let x_is_false: Lit = !x_is_true;
/// let y = state.new_var(0, 10);
/// let y_geq_5 = Lit::geq(y, 5);
///
/// // the `<=` is internally converted into a `<`
/// assert_eq!(y_geq_5.variable(), y);
/// assert_eq!(y_geq_5.relation(), Relation::Gt);
/// assert_eq!(y_geq_5.value(), 4);
/// assert_eq!(y_geq_5.unpack(), (y, Relation::Gt, 4));
/// ```
///
/// # Ordering
///
/// `Lit` defines a very specific order, which is equivalent to sorting the result of the `unpack()` method.
/// The different fields are compared in the following order to define the ordering:
///  - variable
///  - relation
///  - value
///
/// As a result, ordering a vector of `Lit`s will group them by variable, then among literals on the same variable by relation.
/// An important invariant is that, in a sorted list, a bound can only entail the literals immediately following it.
///
/// ```
/// use aries_core::*;
/// let x = VarRef::from_u32(1);
/// let y = VarRef::from_u32(2);
/// let mut literals = vec![Lit::geq(y, 4), Lit::geq(x,1), Lit::leq(x, 3), Lit::leq(x, 4), Lit::leq(x, 6), Lit::geq(x,2)];
/// literals.sort();
/// assert_eq!(literals, vec![Lit::geq(x,2), Lit::geq(x,1), Lit::leq(x, 3), Lit::leq(x, 4), Lit::leq(x, 6), Lit::geq(y, 4)]);
/// ```
#[derive(Copy, Clone, Eq, PartialEq, Hash, Ord, PartialOrd)]
pub struct Lit {
    /// Either the lower bound or the upper bound ot hte affected variable.
    ///
    /// Implemented as the union of the variable (highest 31 bits) and the relation (lowest bit)
    /// This encoding allows:
    ///  - to very efficiently check whether two literals have the same `(variable, relation)` part
    ///    which is one of the critical operation in `entails`.
    ///  - to use as an index in a table: each variable will have two slots: one of the LEQ relation
    ///    and one for the GT relation
    var_bound: VarBound,
    /// +/- the value of the relation. The value of a GT relation is negated before being stored.
    /// This design allows to test entailment without testing the relation of the Bound
    raw_value: BoundValue,
}

#[derive(Ord, PartialOrd, Eq, PartialEq, Debug, Copy, Clone)]
pub enum Relation {
    Gt,
    Leq,
}

impl std::fmt::Display for Relation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Relation::Leq => write!(f, "<="),
            Relation::Gt => write!(f, ">"),
        }
    }
}

impl Lit {
    /// A literal that is always true. It is defined by stating that the special variable [VarRef::ZERO] is
    /// lesser than or equal to 0, which is always true.
    pub const TRUE: Lit = Lit::new(VarRef::ZERO, Relation::Leq, 0);
    /// A literal that is always false. It is defined as the negation of [Lit::TRUE].
    pub const FALSE: Lit = Lit::TRUE.not();

    #[inline]
    pub const fn from_parts(var_bound: VarBound, value: BoundValue) -> Self {
        Lit {
            var_bound,
            raw_value: value,
        }
    }

    #[inline]
    pub const fn new(variable: VarRef, relation: Relation, value: IntCst) -> Self {
        match relation {
            Relation::Leq => Lit {
                var_bound: VarBound::ub(variable),
                raw_value: BoundValue::ub(value),
            },
            Relation::Gt => Lit {
                var_bound: VarBound::lb(variable),
                raw_value: BoundValue::lb(value + 1),
            },
        }
    }

    #[inline]
    pub fn variable(self) -> VarRef {
        self.var_bound.variable()
    }

    #[inline]
    pub const fn relation(self) -> Relation {
        if self.var_bound.is_ub() {
            Relation::Leq
        } else {
            Relation::Gt
        }
    }

    #[inline]
    pub const fn value(self) -> IntCst {
        match self.relation() {
            Relation::Leq => self.raw_value.as_ub(),
            Relation::Gt => self.raw_value.as_lb() - 1,
        }
    }

    #[inline]
    pub const fn affected_bound(self) -> VarBound {
        self.var_bound
    }

    #[inline]
    pub const fn bound_value(self) -> BoundValue {
        self.raw_value
    }

    #[inline]
    pub fn leq(var: impl Into<VarRef>, val: IntCst) -> Lit {
        Lit::new(var.into(), Relation::Leq, val)
    }
    #[inline]
    pub fn lt(var: impl Into<VarRef>, val: IntCst) -> Lit {
        Lit::leq(var, val - 1)
    }

    #[inline]
    pub fn geq(var: impl Into<VarRef>, val: IntCst) -> Lit {
        Lit::gt(var, val - 1)
    }

    #[inline]
    pub fn gt(var: impl Into<VarRef>, val: IntCst) -> Lit {
        Lit::new(var.into(), Relation::Gt, val)
    }

    #[inline]
    pub const fn not(self) -> Self {
        Lit {
            var_bound: self.var_bound.symmetric_bound(),
            raw_value: self.raw_value.neg(),
        }
    }

    #[inline]
    pub fn entails(self, other: Lit) -> bool {
        self.var_bound == other.var_bound && self.raw_value.stronger(other.raw_value)
    }

    pub fn unpack(self) -> (VarRef, Relation, IntCst) {
        (self.variable(), self.relation(), self.value())
    }

    /// An ordering that will group literals by (given from highest to lowest priority):
    ///  - variable
    ///  - affected bound (lower, upper)
    ///  - by value of the bound
    pub fn lexical_cmp(&self, other: &Lit) -> Ordering {
        self.cmp(other)
    }
}

impl std::ops::Not for Lit {
    type Output = Lit;

    #[inline]
    fn not(self) -> Self::Output {
        self.not()
    }
}

impl From<bool> for Lit {
    fn from(b: bool) -> Self {
        if b {
            Lit::TRUE
        } else {
            Lit::FALSE
        }
    }
}

impl std::fmt::Debug for Lit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match *self {
            Lit::TRUE => write!(f, "true"),
            Lit::FALSE => write!(f, "false"),
            _ => {
                let (var, rel, val) = self.unpack();
                if rel == Relation::Gt && val == 0 {
                    write!(f, "l{}", var.to_u32())
                } else if rel == Relation::Leq && val == 0 {
                    write!(f, "!l{}", var.to_u32())
                } else {
                    write!(f, "{var:?} {rel} {val}")
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn leq(var: VarRef, val: IntCst) -> Lit {
        Lit::leq(var, val)
    }
    fn geq(var: VarRef, val: IntCst) -> Lit {
        Lit::geq(var, val)
    }

    #[test]
    fn test_entailments() {
        let a = VarRef::from(0usize);
        let b = VarRef::from(1usize);

        assert!(leq(a, 0).entails(leq(a, 0)));
        assert!(leq(a, 0).entails(leq(a, 1)));
        assert!(!leq(a, 0).entails(leq(a, -1)));

        assert!(!leq(a, 0).entails(leq(b, 0)));
        assert!(!leq(a, 0).entails(leq(b, 1)));
        assert!(!leq(a, 0).entails(leq(b, -1)));

        assert!(geq(a, 0).entails(geq(a, 0)));
        assert!(!geq(a, 0).entails(geq(a, 1)));
        assert!(geq(a, 0).entails(geq(a, -1)));

        assert!(!geq(a, 0).entails(geq(b, 0)));
        assert!(!geq(a, 0).entails(geq(b, 1)));
        assert!(!geq(a, 0).entails(geq(b, -1)));
    }
}
