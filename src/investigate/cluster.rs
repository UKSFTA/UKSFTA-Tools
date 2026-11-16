//! Request clustering by strongest identity phrase.
//!
//! A cluster is a request optimisation only: the unit of record stays
//! the PBO. PBOs that share a normalised strongest phrase share one
//! query; a PBO with no usable phrase forms no cluster.

use std::collections::BTreeMap;

use super::identity::{IdentityEvidence, IdentityPhrase};
use super::normalise::normalise;

/// PBOs that share one strongest phrase.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Cluster {
    pub key: String,
    pub phrase: IdentityPhrase,
    pub members: Vec<String>,
    /// Each member's evidence, in the same order as `members`.
    pub evidence: Vec<IdentityEvidence>,
}

/// One cluster per normalised strongest phrase, ordered by key, with
/// members ordered by PBO file name.
pub(crate) fn cluster_by_identity(evidence: &[IdentityEvidence]) -> Vec<Cluster> {
    let mut clusters: BTreeMap<String, Cluster> = BTreeMap::new();

    for item in evidence {
        let Some(phrase) = item.phrases.first() else {
            continue;
        };
        let key = normalise(&phrase.text).squashed;
        if key.is_empty() {
            continue;
        }
        let cluster = clusters.entry(key.clone()).or_insert_with(|| Cluster {
            key,
            phrase: phrase.clone(),
            members: Vec::new(),
            evidence: Vec::new(),
        });
        cluster.members.push(item.pbo.clone());
        cluster.evidence.push(item.clone());
    }

    let mut clusters: Vec<Cluster> = clusters.into_values().collect();
    for cluster in &mut clusters {
        let mut pairs: Vec<(String, IdentityEvidence)> = cluster
            .members
            .drain(..)
            .zip(cluster.evidence.drain(..))
            .collect();
        pairs.sort_by(|left, right| left.0.cmp(&right.0));
        for (pbo, evidence) in pairs {
            cluster.members.push(pbo);
            cluster.evidence.push(evidence);
        }
    }
    clusters
}

#[cfg(test)]
mod tests {
    use super::super::identity::PhraseSource;
    use super::*;

    fn evidence(pbo: &str, phrase: &str) -> IdentityEvidence {
        IdentityEvidence {
            pbo: pbo.to_string(),
            phrases: vec![IdentityPhrase {
                text: phrase.to_string(),
                source: PhraseSource::CfgModsName,
                workshop_id: None,
            }],
            ..IdentityEvidence::default()
        }
    }

    #[test]
    fn cluster_uses_strongest_phrase_only() {
        let mut first = evidence("b.pbo", "BettIR Core");
        first.phrases.push(IdentityPhrase {
            text: "bettir".to_string(),
            source: PhraseSource::PrefixFamily,
            workshop_id: None,
        });
        let second = evidence("a.pbo", "BettIR Core");

        let clusters = cluster_by_identity(&[first, second]);

        assert_eq!(clusters.len(), 1);
        assert_eq!(clusters[0].phrase.source, PhraseSource::CfgModsName);
        assert_eq!(clusters[0].key, "bettircore");
    }

    #[test]
    fn cluster_members_and_clusters_are_sorted() {
        let items = vec![
            evidence("z.pbo", "Zulu Pack"),
            evidence("b.pbo", "BettIR Core"),
            evidence("a.pbo", "BettIR Core"),
        ];

        let clusters = cluster_by_identity(&items);

        assert_eq!(clusters.len(), 2);
        assert_eq!(clusters[0].key, "bettircore");
        assert_eq!(clusters[0].members, ["a.pbo", "b.pbo"]);
        assert_eq!(clusters[1].key, "zulupack");
        assert_eq!(clusters[1].members, ["z.pbo"]);
    }

    #[test]
    fn cluster_skips_evidence_without_phrase() {
        let clusters = cluster_by_identity(&[IdentityEvidence::default()]);

        assert!(clusters.is_empty());
    }

    #[test]
    fn cluster_shares_one_entry_for_equal_phrases() {
        let items = vec![
            evidence("one.pbo", "BettIR Core"),
            evidence("two.pbo", "bettir core"),
        ];

        let clusters = cluster_by_identity(&items);

        assert_eq!(clusters.len(), 1);
        assert_eq!(clusters[0].members, ["one.pbo", "two.pbo"]);
    }
}
