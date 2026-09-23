//! Builds the format graph from the registry and finds the cheapest path.

// pub mod execute;
// pub mod pipe;

use std::fmt;

use crate::converter::{Converter, FidelityKind};
use crate::event::Hop;
use crate::format::Format;

// pub use execute::execute;

pub struct Plan {
    pub hops: Vec<&'static dyn Converter>,
}

impl fmt::Debug for Plan {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let names: Vec<&'static str> = self.hops.iter().map(|hop| hop.name()).collect();
        formatter
            .debug_struct("Plan")
            .field("hops", &names)
            .finish()
    }
}

impl Plan {
    pub fn from(&self) -> &'static Format {
        self.hops[0].from()
    }

    pub fn to(&self) -> &'static Format {
        let last_index = self.hops.len() - 1;
        self.hops[last_index].to()
    }

    pub fn worst_fidelity(&self) -> FidelityKind {
        let mut worst = FidelityKind::Lossless;
        for hop in &self.hops {
            let kind = hop.fidelity().kind();
            if kind > worst {
                worst = kind;
            }
        }
        worst
    }

    pub fn is_lossless(&self) -> bool {
        self.worst_fidelity() == FidelityKind::Lossless
    }

    pub fn loss_descriptions(&self) -> Vec<&'static str> {
        let mut descriptions = Vec::new();
        for hop in &self.hops {
            if let Some(text) = hop.fidelity().description() {
                descriptions.push(text);
            }
        }
        descriptions
    }

    pub fn describe(&self) -> Vec<Hop> {
        let mut hops = Vec::with_capacity(self.hops.len());
        for converter in &self.hops {
            let hop = Hop {
                converter: converter.name(),
                from: converter.from().id,
                to: converter.to().id,
                fidelity: converter.fidelity(),
                tier: converter.tier(),
            };
            hops.push(hop);
        }
        hops
    }
}

#[derive(Debug, Clone, Default)]
pub struct PlanOptions {
    pub strict: bool,
    pub via: Option<&'static Format>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum PlanError {
    SameFormat(&'static str),
    NoPath {
        from: &'static str,
        to: &'static str,
        pruned: Vec<&'static str>,
    },
}

impl fmt::Display for PlanError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PlanError::SameFormat(id) => {
                write!(
                    formatter,
                    "input and output are both '{id}'; nothing to convert"
                )
            }
            PlanError::NoPath { from, to, pruned } => {
                if pruned.is_empty() {
                    write!(formatter, "no conversion path from '{from}' to '{to}'")
                } else {
                    let names = pruned.join(", ");
                    write!(
                        formatter,
                        "no lossless path from '{from}' to '{to}'; a path exists through lossy or conditional converters ({names}); drop --strict to allow it"
                    )
                }
            }
        }
    }
}

impl std::error::Error for PlanError {}

pub fn plan(
    converters: &[&'static dyn Converter],
    from: &'static Format,
    to: &'static Format,
    options: &PlanOptions,
) -> Result<Plan, PlanError> {
    if from == to {
        return Err(PlanError::SameFormat(from.id));
    }
    let via = match options.via {
        Some(via) => via,
        None => return search(converters, from, to, options.strict),
    };
    let via_is_endpoint = via == from || via == to;
    if via_is_endpoint {
        return search(converters, from, to, options.strict);
    }
    let first_leg = search(converters, from, via, options.strict)?;
    let second_leg = search(converters, via, to, options.strict)?;
    let mut hops = first_leg.hops;
    hops.extend(second_leg.hops);
    Ok(Plan { hops })
}

fn search(
    converters: &[&'static dyn Converter],
    from: &'static Format,
    to: &'static Format,
    strict: bool,
) -> Result<Plan, PlanError> {
    let unrestricted = dijkstra(converters, from, to, false);
    if !strict {
        return match unrestricted {
            Some(plan) => Ok(plan),
            None => Err(PlanError::NoPath {
                from: from.id,
                to: to.id,
                pruned: Vec::new(),
            }),
        };
    }
    let restricted = dijkstra(converters, from, to, true);
    if let Some(plan) = restricted {
        return Ok(plan);
    }
    let mut pruned = Vec::new();
    if let Some(plan) = unrestricted {
        for hop in plan.hops {
            if !hop.fidelity().is_lossless() {
                pruned.push(hop.name());
            }
        }
    }
    Err(PlanError::NoPath {
        from: from.id,
        to: to.id,
        pruned,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PathCost {
    cost: u32,
    hops: u32,
    tier: u32,
}

impl PathCost {
    fn key(&self) -> (u32, u32, u32) {
        (self.cost, self.hops, self.tier)
    }
}

/// Readable Dijkstra over a graph of at most a few hundred formats.
fn dijkstra(
    converters: &[&'static dyn Converter],
    from: &'static Format,
    to: &'static Format,
    lossless_only: bool,
) -> Option<Plan> {
    let mut node_ids: Vec<&'static str> = Vec::new();
    push_node(&mut node_ids, from.id);
    push_node(&mut node_ids, to.id);
    let mut edges: Vec<&'static dyn Converter> = Vec::new();
    for converter in converters {
        let allowed = !lossless_only || converter.fidelity().is_lossless();
        if !allowed {
            continue;
        }
        push_node(&mut node_ids, converter.from().id);
        push_node(&mut node_ids, converter.to().id);
        edges.push(*converter);
    }

    let node_count = node_ids.len();
    let source = index_of(&node_ids, from.id);
    let target = index_of(&node_ids, to.id);
    let mut best: Vec<Option<PathCost>> = vec![None; node_count];
    let mut incoming_edge: Vec<Option<usize>> = vec![None; node_count];
    let mut visited: Vec<bool> = vec![false; node_count];
    best[source] = Some(PathCost {
        cost: 0,
        hops: 0,
        tier: 0,
    });

    while let Some(current) = pick_next(&best, &visited) {
        if current == target {
            break;
        }
        visited[current] = true;
        let current_cost = match best[current] {
            Some(cost) => cost,
            None => break,
        };
        for (edge_index, converter) in edges.iter().enumerate() {
            let edge_source = index_of(&node_ids, converter.from().id);
            if edge_source != current {
                continue;
            }
            let neighbor = index_of(&node_ids, converter.to().id);
            let candidate = PathCost {
                cost: current_cost.cost + converter.fidelity().cost(),
                hops: current_cost.hops + 1,
                tier: current_cost.tier + converter.tier().rank(),
            };
            let is_better = match best[neighbor] {
                None => true,
                Some(existing) => {
                    let existing_name = incoming_name(&edges, incoming_edge[neighbor]);
                    beats(candidate, existing, converter.name(), existing_name)
                }
            };
            if is_better {
                best[neighbor] = Some(candidate);
                incoming_edge[neighbor] = Some(edge_index);
            }
        }
    }

    best[target]?;
    let mut hops: Vec<&'static dyn Converter> = Vec::new();
    let mut cursor = target;
    while cursor != source {
        let edge_index = incoming_edge[cursor]?;
        let converter = edges[edge_index];
        hops.push(converter);
        cursor = index_of(&node_ids, converter.from().id);
    }
    hops.reverse();
    Some(Plan { hops })
}

fn push_node(node_ids: &mut Vec<&'static str>, id: &'static str) {
    let already_present = node_ids.contains(&id);
    if !already_present {
        node_ids.push(id);
    }
}

fn index_of(node_ids: &[&'static str], id: &str) -> usize {
    for (index, existing) in node_ids.iter().enumerate() {
        if *existing == id {
            return index;
        }
    }
    0
}

fn pick_next(best: &[Option<PathCost>], visited: &[bool]) -> Option<usize> {
    let mut chosen: Option<usize> = None;
    for (index, candidate) in best.iter().enumerate() {
        if visited[index] {
            continue;
        }
        let candidate_cost = match candidate {
            Some(cost) => cost,
            None => continue,
        };
        let is_better = match chosen {
            None => true,
            Some(chosen_index) => match best[chosen_index] {
                Some(chosen_cost) => candidate_cost.key() < chosen_cost.key(),
                None => true,
            },
        };
        if is_better {
            chosen = Some(index);
        }
    }
    chosen
}

fn incoming_name(edges: &[&'static dyn Converter], edge_index: Option<usize>) -> &'static str {
    match edge_index {
        Some(index) => edges[index].name(),
        None => "",
    }
}

fn beats(
    candidate: PathCost,
    existing: PathCost,
    candidate_name: &str,
    existing_name: &str,
) -> bool {
    if candidate.key() != existing.key() {
        return candidate.key() < existing.key();
    }
    candidate_name < existing_name
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::converter::{ConvertError, Fidelity, Input, Tier};
    use crate::event::Context;
    use std::io::Write;

    static A: Format = Format {
        id: "a",
        display_name: "A",
        extensions: &["a"],
        magic: None,
    };
    static B: Format = Format {
        id: "b",
        display_name: "B",
        extensions: &["b"],
        magic: None,
    };
    static C: Format = Format {
        id: "c",
        display_name: "C",
        extensions: &["c"],
        magic: None,
    };
    static D: Format = Format {
        id: "d",
        display_name: "D",
        extensions: &["d"],
        magic: None,
    };

    struct Fake {
        name: &'static str,
        from: &'static Format,
        to: &'static Format,
        fidelity: Fidelity,
        tier: Tier,
    }

    impl Converter for Fake {
        fn name(&self) -> &'static str {
            self.name
        }
        fn from(&self) -> &'static Format {
            self.from
        }
        fn to(&self) -> &'static Format {
            self.to
        }
        fn fidelity(&self) -> Fidelity {
            self.fidelity.clone()
        }
        fn tier(&self) -> Tier {
            self.tier
        }
        fn convert(
            &self,
            _input: Input<'_>,
            _output: &mut dyn Write,
            _context: &mut Context<'_>,
        ) -> Result<(), ConvertError> {
            Err(ConvertError::Unsupported("fake".to_string()))
        }
    }

    static A_B: Fake = Fake {
        name: "a-b",
        from: &A,
        to: &B,
        fidelity: Fidelity::Lossless,
        tier: Tier::Native,
    };
    static B_C: Fake = Fake {
        name: "b-c",
        from: &B,
        to: &C,
        fidelity: Fidelity::Lossless,
        tier: Tier::Native,
    };
    static C_D: Fake = Fake {
        name: "c-d",
        from: &C,
        to: &D,
        fidelity: Fidelity::Lossless,
        tier: Tier::Native,
    };
    static A_D_LOSSY: Fake = Fake {
        name: "a-d-lossy",
        from: &A,
        to: &D,
        fidelity: Fidelity::Lossy("drops"),
        tier: Tier::Native,
    };
    static A_C_COND: Fake = Fake {
        name: "a-c-cond",
        from: &A,
        to: &C,
        fidelity: Fidelity::Conditional("maybe"),
        tier: Tier::Native,
    };
    static A_B_EXTERNAL: Fake = Fake {
        name: "a-b-ext",
        from: &A,
        to: &B,
        fidelity: Fidelity::Lossless,
        tier: Tier::External,
    };

    fn names(plan: &Plan) -> Vec<&'static str> {
        plan.hops.iter().map(|hop| hop.name()).collect()
    }

    #[test]
    fn prefers_three_lossless_hops_over_one_lossy_hop() {
        let converters: Vec<&'static dyn Converter> = vec![&A_B, &B_C, &C_D, &A_D_LOSSY];
        let plan = plan(&converters, &A, &D, &PlanOptions::default()).unwrap();
        assert_eq!(names(&plan), vec!["a-b", "b-c", "c-d"]);
        assert!(plan.is_lossless());
    }

    #[test]
    fn takes_the_lossy_hop_when_it_is_the_only_way() {
        let converters: Vec<&'static dyn Converter> = vec![&A_D_LOSSY];
        let plan = plan(&converters, &A, &D, &PlanOptions::default()).unwrap();
        assert_eq!(names(&plan), vec!["a-d-lossy"]);
        assert_eq!(plan.worst_fidelity(), FidelityKind::Lossy);
        assert_eq!(plan.loss_descriptions(), vec!["drops"]);
    }

    #[test]
    fn strict_refuses_lossy_and_names_the_pruned_edges() {
        let converters: Vec<&'static dyn Converter> = vec![&A_D_LOSSY, &A_C_COND, &C_D];
        let options = PlanOptions {
            strict: true,
            via: None,
        };
        let error = plan(&converters, &A, &D, &options).unwrap_err();
        match error {
            PlanError::NoPath { pruned, .. } => assert_eq!(pruned, vec!["a-c-cond"]),
            other => panic!("wrong error {other:?}"),
        }
    }

    #[test]
    fn via_forces_a_waypoint() {
        let converters: Vec<&'static dyn Converter> = vec![&A_B, &B_C, &C_D, &A_D_LOSSY, &A_C_COND];
        let options = PlanOptions {
            strict: false,
            via: Some(&C),
        };
        let plan = plan(&converters, &A, &D, &options).unwrap();
        assert_eq!(names(&plan), vec!["a-b", "b-c", "c-d"]);
    }

    #[test]
    fn via_equal_to_an_endpoint_is_ignored() {
        let converters: Vec<&'static dyn Converter> = vec![&A_B, &B_C];
        let via_from = PlanOptions {
            strict: false,
            via: Some(&A),
        };
        let via_to = PlanOptions {
            strict: false,
            via: Some(&C),
        };
        let first = plan(&converters, &A, &C, &via_from).unwrap();
        let second = plan(&converters, &A, &C, &via_to).unwrap();
        assert_eq!(names(&first), vec!["a-b", "b-c"]);
        assert_eq!(names(&second), vec!["a-b", "b-c"]);
    }

    #[test]
    fn no_path_is_an_error() {
        let converters: Vec<&'static dyn Converter> = vec![&A_B];
        let error = plan(&converters, &A, &D, &PlanOptions::default()).unwrap_err();
        let is_no_path = matches!(error, PlanError::NoPath { .. });
        assert!(is_no_path);
    }

    #[test]
    fn same_format_is_an_error() {
        let converters: Vec<&'static dyn Converter> = vec![&A_B];
        let error = plan(&converters, &A, &A, &PlanOptions::default()).unwrap_err();
        assert_eq!(error, PlanError::SameFormat("a"));
    }

    #[test]
    fn native_beats_external_on_a_tie() {
        let converters: Vec<&'static dyn Converter> = vec![&A_B_EXTERNAL, &A_B];
        let plan = plan(&converters, &A, &B, &PlanOptions::default()).unwrap();
        assert_eq!(names(&plan), vec!["a-b"]);
    }

    #[test]
    fn describe_lists_hops() {
        let converters: Vec<&'static dyn Converter> = vec![&A_B];
        let plan = plan(&converters, &A, &B, &PlanOptions::default()).unwrap();
        let hops = plan.describe();
        assert_eq!(hops.len(), 1);
        assert_eq!(hops[0].converter, "a-b");
        assert_eq!(hops[0].from, "a");
        assert_eq!(hops[0].to, "b");
    }
}
