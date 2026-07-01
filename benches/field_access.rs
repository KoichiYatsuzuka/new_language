// benches/field_access.rs
//
// Compares two field-access strategies for Arrow class instances:
//
// 1. CURRENT  — InstanceData.fields: HashMap<String, (Value, bool)>
//               Access: inst.fields.get("field_name")
//
// 2. PROPOSED — InstanceData.fields: Vec<(Value, bool)>   (after optimization)
//               ClassValue.field_index: HashMap<String, usize>  (shared, per-class)
//               Access: inst.fields[class.field_index["field_name"]]
//
// The benchmark isolates the data-structure cost from the rest of the interpreter.
// Each test performs N repeated reads of a 4-field struct-like record.

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Shared value type (simplified — only the data-structure layer is measured)
// ---------------------------------------------------------------------------

#[derive(Clone)]
enum Val {
    Float(f64),
}

// ---------------------------------------------------------------------------
// CURRENT approach: per-instance HashMap
// ---------------------------------------------------------------------------

struct InstanceHashMap {
    fields: HashMap<String, (Val, bool)>,
}

fn make_hashmap_instance(x: f64, y: f64, z: f64, mass: f64) -> InstanceHashMap {
    let mut fields = HashMap::with_capacity(4);
    fields.insert("x".to_string(),    (Val::Float(x),    true));
    fields.insert("y".to_string(),    (Val::Float(y),    true));
    fields.insert("z".to_string(),    (Val::Float(z),    true));
    fields.insert("mass".to_string(), (Val::Float(mass), true));
    InstanceHashMap { fields }
}

#[inline(never)]
fn access_hashmap(inst: &InstanceHashMap) -> f64 {
    let Val::Float(x)    = inst.fields["x"].0    else { unreachable!() };
    let Val::Float(y)    = inst.fields["y"].0    else { unreachable!() };
    let Val::Float(z)    = inst.fields["z"].0    else { unreachable!() };
    let Val::Float(mass) = inst.fields["mass"].0 else { unreachable!() };
    mass * (x * x + y * y + z * z)
}

// ---------------------------------------------------------------------------
// PROPOSED approach: shared ClassValue with field_index + per-instance Vec
// ---------------------------------------------------------------------------

struct ClassValue {
    field_index: HashMap<String, usize>,
}

struct InstanceVec {
    fields: Vec<(Val, bool)>,
}

fn make_class_value() -> ClassValue {
    let mut fi = HashMap::with_capacity(4);
    fi.insert("x".to_string(),    0);
    fi.insert("y".to_string(),    1);
    fi.insert("z".to_string(),    2);
    fi.insert("mass".to_string(), 3);
    ClassValue { field_index: fi }
}

fn make_vec_instance(x: f64, y: f64, z: f64, mass: f64) -> InstanceVec {
    InstanceVec {
        fields: vec![
            (Val::Float(x),    true),
            (Val::Float(y),    true),
            (Val::Float(z),    true),
            (Val::Float(mass), true),
        ],
    }
}

#[inline(never)]
fn access_vec_dynamic(inst: &InstanceVec, cls: &ClassValue) -> f64 {
    // Still looks up field_index (needed for trait-typed refs); shared ClassValue.
    let ix = cls.field_index["x"];
    let iy = cls.field_index["y"];
    let iz = cls.field_index["z"];
    let im = cls.field_index["mass"];
    let Val::Float(x)    = inst.fields[ix].0 else { unreachable!() };
    let Val::Float(y)    = inst.fields[iy].0 else { unreachable!() };
    let Val::Float(z)    = inst.fields[iz].0 else { unreachable!() };
    let Val::Float(mass) = inst.fields[im].0 else { unreachable!() };
    mass * (x * x + y * y + z * z)
}

#[inline(never)]
fn access_vec_static(inst: &InstanceVec) -> f64 {
    // Concrete type known at compile time → pure integer indices, no HashMap.
    let Val::Float(x)    = inst.fields[0].0 else { unreachable!() };
    let Val::Float(y)    = inst.fields[1].0 else { unreachable!() };
    let Val::Float(z)    = inst.fields[2].0 else { unreachable!() };
    let Val::Float(mass) = inst.fields[3].0 else { unreachable!() };
    mass * (x * x + y * y + z * z)
}

// ---------------------------------------------------------------------------
// Benchmarks
// ---------------------------------------------------------------------------

const N: usize = 10_000;

fn bench_single_instance(c: &mut Criterion) {
    let hm_inst = make_hashmap_instance(1.0, 2.0, 3.0, 1.5);
    let cls     = make_class_value();
    let v_inst  = make_vec_instance(1.0, 2.0, 3.0, 1.5);

    let mut group = c.benchmark_group("single_instance_field_read");
    group.bench_function("hashmap (current)", |b| {
        b.iter(|| black_box(access_hashmap(black_box(&hm_inst))))
    });
    group.bench_function("vec+class_field_index (trait-typed)", |b| {
        b.iter(|| black_box(access_vec_dynamic(black_box(&v_inst), black_box(&cls))))
    });
    group.bench_function("vec+static_index (concrete type known)", |b| {
        b.iter(|| black_box(access_vec_static(black_box(&v_inst))))
    });
    group.finish();
}

fn bench_loop_n_instances(c: &mut Criterion) {
    let hm_insts: Vec<_> = (0..N)
        .map(|i| { let f = i as f64 * 0.001; make_hashmap_instance(f, f*2.0, f*3.0, 1.0+f) })
        .collect();

    let cls = make_class_value();
    let v_insts: Vec<_> = (0..N)
        .map(|i| { let f = i as f64 * 0.001; make_vec_instance(f, f*2.0, f*3.0, 1.0+f) })
        .collect();

    let mut group = c.benchmark_group("loop_N_instances");
    group.bench_function(
        BenchmarkId::new("hashmap (current)", N),
        |b| b.iter(|| {
            let mut sum = 0.0_f64;
            for inst in &hm_insts { sum += access_hashmap(inst); }
            black_box(sum)
        }),
    );
    group.bench_function(
        BenchmarkId::new("vec+class_field_index (trait-typed)", N),
        |b| b.iter(|| {
            let mut sum = 0.0_f64;
            for inst in &v_insts { sum += access_vec_dynamic(inst, &cls); }
            black_box(sum)
        }),
    );
    group.bench_function(
        BenchmarkId::new("vec+static_index (concrete type known)", N),
        |b| b.iter(|| {
            let mut sum = 0.0_f64;
            for inst in &v_insts { sum += access_vec_static(inst); }
            black_box(sum)
        }),
    );
    group.finish();
}

criterion_group!(benches, bench_single_instance, bench_loop_n_instances);
criterion_main!(benches);
