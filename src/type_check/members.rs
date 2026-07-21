// type_check/members.rs — 型のメンバー解決と Intersection 型の適合検査。
//
// Phase 5A-4 で mod.rs から移設（MemberKind / get_type_members /
// check_intersection_members / check_intersection_guard_type /
// class_implements_protocol）。mod.rs をファサードに保つための分離であり、
// ロジックは移設前と同一。

use std::collections::{HashMap, HashSet};

use crate::ast::{Accessibility, FieldKind};

use super::errors;
use super::types::{FnSig, InferredType};
use super::TypeChecker;

// ---------------------------------------------------------------------------
// Intersection type helpers
// ---------------------------------------------------------------------------

/// メンバーを表す内部型。フィールドかメソッドのいずれか。
#[derive(Clone, Debug)]
pub(crate) enum MemberKind {
    Field {
        kind: FieldKind,
        ty: InferredType,
        access: Accessibility,
    },
    Method {
        sigs: Vec<FnSig>,
    },
}

impl TypeChecker {
    /// 型から公開メンバーの名前→MemberKind マップを収集する。
    pub(crate) fn get_type_members(&self, ty: &InferredType) -> HashMap<String, MemberKind> {
        let mut members: HashMap<String, MemberKind> = HashMap::new();
        match ty {
            InferredType::NamedInstance(name) => {
                // Class fields
                if let Some(fields) = self.registry.class_field_details(name.as_str()) {
                    for (fname, (kind, fty)) in fields {
                        let access = self.registry.member_access(name.as_str(), fname);
                        members.insert(fname.clone(), MemberKind::Field {
                            kind: kind.clone(),
                            ty: fty.clone(),
                            access,
                        });
                    }
                }
                // Class methods
                if let Some(methods) = self.registry.class_methods(name.as_str()) {
                    for (mname, sigs) in methods {
                        members.insert(mname.clone(), MemberKind::Method {
                            sigs: sigs.clone(),
                        });
                    }
                }
                // Trait fields
                if let Some(fields) = self.registry.trait_field_details(name.as_str()) {
                    for (fname, (kind, fty)) in fields {
                        members.entry(fname.clone()).or_insert(MemberKind::Field {
                            kind: kind.clone(),
                            ty: fty.clone(),
                            access: Accessibility::Public,
                        });
                    }
                }
                // Trait methods
                if let Some(methods) = self.registry.trait_methods(name.as_str()) {
                    for (mname, sigs) in methods {
                        members.entry(mname.clone()).or_insert(MemberKind::Method {
                            sigs: sigs.clone(),
                        });
                    }
                }
            }
            InferredType::Protocol(name) => {
                if let Some(proto) = self.registry.protocol(name.as_str()) {
                    for f in &proto.fields {
                        members.insert(f.name.clone(), MemberKind::Field {
                            kind: f.kind.clone(),
                            ty: f.ty.clone(),
                            access: Accessibility::Public,
                        });
                    }
                    for m in &proto.methods {
                        let sig = FnSig {
                            params: m.params.iter()
                                .map(|(pname, _pmut, pty)| (pname.clone(), Some(pty.clone())))
                                .collect(),
                            required_count: m.params.len(),
                            return_type: Some(m.return_type.clone()),
                            variadic_type: None,
                        };
                        members.insert(m.name.clone(), MemberKind::Method {
                            sigs: vec![sig],
                        });
                    }
                }
            }
            _ => {}
        }
        members
    }

    /// 交差型の構成型間のメンバー互換性を検査し、重複は警告、競合はエラーを収集する。
    pub(crate) fn check_intersection_members(
        &mut self,
        types: &[InferredType],
        span: Option<crate::token::Span>,
    ) {
        use errors::{StaticTypeError, StaticTypeWarning, TypeErrorKind, TypeWarningKind};

        if types.len() < 2 {
            return;
        }
        // Collect member maps for each constituent type
        let member_maps: Vec<(String, HashMap<String, MemberKind>)> = types.iter()
            .map(|t| (t.to_string(), self.get_type_members(t)))
            .collect();

        // For each unique member name present in more than one type, compare
        let all_names: HashSet<&String> = member_maps.iter()
            .flat_map(|(_, m)| m.keys())
            .collect();

        for name in all_names {
            // Skip self / cls
            if name == "self" || name == "cls" || name == "__init__" {
                continue;
            }
            let entries: Vec<(&String, &MemberKind)> = member_maps.iter()
                .filter_map(|(tname, m)| m.get(name).map(|mk| (tname, mk)))
                .collect();
            if entries.len() < 2 {
                continue;
            }
            // Compare first entry against subsequent ones
            let (type_a, mk_a) = entries[0];
            for (type_b, mk_b) in &entries[1..] {
                match (mk_a, mk_b) {
                    (MemberKind::Field { kind: ka, ty: ta, access: acc_a },
                     MemberKind::Field { kind: kb, ty: tb, access: acc_b }) => {
                        let type_mismatch = ta != tb;
                        let access_mismatch = acc_a != acc_b;
                        let kind_mismatch = ka != kb;
                        if type_mismatch || access_mismatch || kind_mismatch {
                            let reason = if type_mismatch {
                                format!("field type differs: {} vs {}", ta, tb)
                            } else if access_mismatch {
                                format!("access modifier differs: {:?} vs {:?}", acc_a, acc_b)
                            } else {
                                format!("field qualifier differs: {:?} vs {:?}", ka, kb)
                            };
                            self.report_error(StaticTypeError {
                                kind: TypeErrorKind::IntersectionMemberConflict {
                                    member_name: name.clone(),
                                    type_a: type_a.clone(),
                                    type_b: type_b.to_string(),
                                    reason,
                                },
                                span: span.clone(),
                            });
                        } else {
                            self.report_warning(StaticTypeWarning {
                                kind: TypeWarningKind::IntersectionMemberDuplicate {
                                    member_name: name.clone(),
                                    type_a: type_a.clone(),
                                    type_b: type_b.to_string(),
                                },
                                span: span.clone(),
                            });
                        }
                    }
                    (MemberKind::Method { sigs: sigs_a, .. },
                     MemberKind::Method { sigs: sigs_b, .. }) => {
                        // Check if any sig from A is identical to any sig from B (same types)
                        // or if there's a non-overloadable conflict
                        let mut found_conflict = false;
                        let mut found_duplicate = false;
                        'outer: for sig_a in sigs_a {
                            for sig_b in sigs_b {
                                // Compare params (ignoring self)
                                let a_params: Vec<_> = sig_a.params.iter().filter(|(n, _)| n != "self").collect();
                                let b_params: Vec<_> = sig_b.params.iter().filter(|(n, _)| n != "self").collect();
                                if a_params.len() != b_params.len() {
                                    // Different param count → overloadable → warning only (handled below)
                                    continue;
                                }
                                // Same param count: check types
                                let same_types = a_params.iter().zip(b_params.iter())
                                    .all(|((_, ta), (_, tb))| ta == tb);
                                if !same_types {
                                    // Different param types → overloadable → warning only
                                    continue;
                                }
                                // Same param count and types: check if names/mutability differ
                                let same_names = a_params.iter().zip(b_params.iter())
                                    .all(|((na, _), (nb, _))| na == nb);
                                if !same_names {
                                    found_conflict = true;
                                    break 'outer;
                                }
                                // Identical → duplicate warning
                                if sig_a.return_type == sig_b.return_type {
                                    found_duplicate = true;
                                }
                            }
                        }
                        if found_conflict {
                            self.report_error(StaticTypeError {
                                kind: TypeErrorKind::IntersectionMemberConflict {
                                    member_name: name.clone(),
                                    type_a: type_a.clone(),
                                    type_b: type_b.to_string(),
                                    reason: "method signatures have same parameter types but different parameter names (non-overloadable)".to_string(),
                                },
                                span: span.clone(),
                            });
                        } else if found_duplicate {
                            self.report_warning(StaticTypeWarning {
                                kind: TypeWarningKind::IntersectionMemberDuplicate {
                                    member_name: name.clone(),
                                    type_a: type_a.clone(),
                                    type_b: type_b.to_string(),
                                },
                                span: span.clone(),
                            });
                        } else {
                            // Different signatures (overloadable) → warning
                            self.report_warning(StaticTypeWarning {
                                kind: TypeWarningKind::IntersectionMemberDuplicate {
                                    member_name: name.clone(),
                                    type_a: type_a.clone(),
                                    type_b: type_b.to_string(),
                                },
                                span: span.clone(),
                            });
                        }
                    }
                    _ => {
                        // One is a field, the other is a method — conflict
                        self.report_error(StaticTypeError {
                            kind: TypeErrorKind::IntersectionMemberConflict {
                                member_name: name.clone(),
                                type_a: type_a.clone(),
                                type_b: type_b.to_string(),
                                reason: "member is a field in one type and a method in another".to_string(),
                            },
                            span: span.clone(),
                        });
                    }
                }
            }
        }
    }

    /// 型 `guard_type` が交差型 `intersection_types` のすべての構成型を満たすか検査する。
    /// 満たさない場合はエラーを収集して `false` を返す。
    pub(crate) fn check_intersection_guard_type(
        &mut self,
        guard_type: &str,
        intersection_types: &[InferredType],
        span: Option<crate::token::Span>,
    ) -> bool {
        use errors::{StaticTypeError, TypeErrorKind};
        let mut ok = true;
        for ty in intersection_types {
            let ty_name = match ty {
                InferredType::NamedInstance(n) | InferredType::Protocol(n) => n.clone(),
                _ => continue,
            };
            let satisfied = if self.registry.is_protocol(ty_name.as_str()) {
                // Protocol: check conformance via class field/method existence
                self.class_implements_protocol(guard_type, &ty_name)
            } else {
                // Trait or class: check inheritance
                self.class_implements_trait(guard_type, &ty_name) || guard_type == ty_name
            };
            if !satisfied {
                let intersection_str = format!("Intersection[{}]",
                    intersection_types.iter().map(|t| t.to_string()).collect::<Vec<_>>().join(", "));
                self.report_error(StaticTypeError {
                    kind: TypeErrorKind::IntersectionGuardTypeFails {
                        guard_type: guard_type.to_string(),
                        intersection_type: intersection_str,
                        reason: format!("'{}' does not satisfy constraint '{}'", guard_type, ty_name),
                    },
                    span: span.clone(),
                });
                ok = false;
            }
        }
        ok
    }

    /// クラスがプロトコルを満たすかを簡易チェックする（フィールド名・メソッド名の存在確認）。
    fn class_implements_protocol(&self, class_name: &str, protocol_name: &str) -> bool {
        let Some(proto) = self.registry.protocol(protocol_name) else {
            return false;
        };
        let class_fields = self.registry.class_field_details(class_name);
        let class_methods = self.registry.class_methods(class_name);
        for f in &proto.fields {
            if class_fields.is_none_or(|m| !m.contains_key(&f.name)) {
                return false;
            }
        }
        for m in &proto.methods {
            if class_methods.is_none_or(|ms| !ms.contains_key(&m.name)) {
                return false;
            }
        }
        true
    }
}
