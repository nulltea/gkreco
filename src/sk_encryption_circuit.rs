pub use crate::constants::sk_enc_constants_1024_2x55_65537::{
    E_BOUND, K0IS, K1_BOUND, N, QIS, R1_BOUNDS, R2_BOUNDS,
};
use crate::{constants::sk_enc_constants_1024_1x27_65537::S_BOUND, lasso::LassoPreprocessing};
use crate::{
    lasso::{table::range::RangeLookup, LassoNode},
    poly::Poly,
    transcript::Keccak256Transcript,
};
use gkr::circuit::node::LogUpNode;
use gkr::{
    chain_par,
    circuit::{
        connect,
        node::{EvalClaim, FftNode, InputNode, VanillaGate, VanillaNode},
        Circuit, NodeId,
    },
    ff_ext::ff::PrimeField,
    poly::{box_dense_poly, BoxMultilinearPoly},
    transcript::Transcript,
    util::{
        arithmetic::{radix2_fft, squares, ExtensionField},
        izip, Itertools,
    },
    verify_gkr,
};
use itertools::chain;
use paste::paste;
use plonkish_backend::pcs::multilinear::MultilinearBrakedown;
use plonkish_backend::pcs::PolynomialCommitmentScheme;
use plonkish_backend::poly::multilinear::MultilinearPolynomial;
use plonkish_backend::util::code::BrakedownSpec6;
use plonkish_backend::util::hash::{Keccak256, Output};
use rand::RngCore;
use rayon::iter::{IntoParallelRefIterator, ParallelIterator};
use rayon::vec;
use serde::Deserialize;
use std::any::TypeId;
use std::cmp::min;
use std::iter;
use strum_macros::{EnumCount, EnumIter};
use tracing::info_span;

const E_BOUND_LEN: usize = (2 * E_BOUND + 1).next_power_of_two().ilog2() as usize;
const K1_BOUND_LEN: usize = (2 * K1_BOUND + 1).next_power_of_two().ilog2() as usize;
pub const R2_BOUND: u64 = R2_BOUNDS[0];

const LIMB_BITS: usize = 16;
const C: usize = 4;
const M: usize = 1 << LIMB_BITS;

pub type Brakedown<F> =
    MultilinearBrakedown<F, plonkish_backend::util::hash::Keccak256, BrakedownSpec6>;

pub type ProverKey<
    F,
    E,
    Pcs: PolynomialCommitmentScheme<
        F,
        Polynomial = MultilinearPolynomial<F>,
        CommitmentChunk = Output<Keccak256>,
    >,
> = (
    LassoPreprocessing<F, E>,
    Vec<(Pcs::Commitment, Pcs::ProverParam)>,
);

pub type VerifierKey<
    F,
    E,
    Pcs: PolynomialCommitmentScheme<
        F,
        Polynomial = MultilinearPolynomial<F>,
        CommitmentChunk = Output<Keccak256>,
    >,
> = (
    LassoPreprocessing<F, E>,
    Vec<(Pcs::Commitment, Pcs::VerifierParam)>,
);

/// `BfvSkEncryptionCircuit` is a circuit that checks the correct formation of a ciphertext resulting from BFV secret key encryption
/// All the polynomials coefficients and scalars are normalized to be in the range `[0, p)` where p is the modulus of the prime field of the circuit
///
/// # Parameters:
/// * `s`: secret polynomial, sampled from ternary distribution.
/// * `e`: error polynomial, sampled from discrete Gaussian distribution.
/// * `k1`: scaled message polynomial.
/// * `r2is`: list of r2i polynomials for each i-th CRT basis .
/// * `r1is`: list of r1i polynomials for each CRT i-th CRT basis.
/// * `ais`: list of ai polynomials for each CRT i-th CRT basis.
/// * `ct0is`: list of ct0i (first component of the ciphertext cti) polynomials for each CRT i-th CRT basis.
#[derive(Deserialize, Clone)]
pub struct BfvSkEncryptArgs {
    s: Vec<String>,
    e: Vec<String>,
    k1: Vec<String>,
    r2is: Vec<Vec<String>>,
    r1is: Vec<Vec<String>>,
    ais: Vec<Vec<String>>,
    ct0is: Vec<Vec<String>>,
}

pub struct BfvEncryptBlock<const POLY_LOG2_SIZE: usize> {
    num_reps: usize,
}

impl<const POLY_LOG2_SIZE: usize> BfvEncryptBlock<POLY_LOG2_SIZE> {
    pub const fn log2_size(&self) -> usize {
        POLY_LOG2_SIZE + 1
    }

    pub const fn r2i_bound_log2_size() -> usize {
        // (2 * R2_BOUNDS[0] + 1).next_power_of_two().ilog2() as usize
        64
    }

    pub fn r1i_bound_log2_size(&self) -> Vec<usize> {
        R1_BOUNDS
            .into_iter()
            .take(self.num_reps)
            .map(|b| (2 * b + 1).next_power_of_two().ilog2() as usize)
            .collect()
    }

    // single block
    pub fn configure<F: PrimeField, E: ExtensionField<F>>(
        &self,
        circuit: &mut Circuit<F, E>,
        s: NodeId,
        e: NodeId,
        k1: NodeId,
        preprocessing: LassoPreprocessing<F, E>,
    ) -> NodeId {
        let poly_log2_size = POLY_LOG2_SIZE;
        let log2_size = self.log2_size();

        let es = {
            let gates = (0..self.num_reps)
                .flat_map(|_| (0..(1usize << log2_size)).map(move |j| VanillaGate::relay((0, j))))
                .collect_vec();

            circuit.insert(VanillaNode::new(1, log2_size, gates.clone(), 1))
        };

        let k1kis = {
            let gates = (0..self.num_reps)
                .flat_map(|i| {
                    (0..(1usize << log2_size)).map(move |j| {
                        relay_mul_const((0, j), F::from_str_vartime(K0IS[i]).unwrap())
                    })
                })
                .collect_vec();

            circuit.insert(VanillaNode::new(1, log2_size, gates.clone(), 1))
        };

        connect!(circuit {
            es <- e;
            k1kis <- k1;
        });

        let ais = iter::repeat_with(|| circuit.insert(InputNode::new(log2_size, 1)))
            .take(self.num_reps)
            .collect_vec();

        let r1is = iter::repeat_with(|| circuit.insert(InputNode::new(log2_size, 1)))
            .take(self.num_reps)
            .collect_vec();

        // for (i, &r1i) in r1is.iter().enumerate().take(self.num_reps) {
        //     let log2_t_size = self.r1i_bound_log2_size()[i];
        //     let r1i_m = circuit.insert(InputNode::new(log2_t_size, 1));
        //     let r1i_t = circuit.insert(InputNode::new(log2_t_size, 1));
        //     let r1i_range = circuit.insert(LogUpNode::new(log2_t_size, log2_size, 1));

        //     connect!(circuit {
        //         r1i_range <- r1i_m, r1i_t, r1i;
        //     });
        // }

        let r1iqis = {
            let r1i_size = 1usize << log2_size;
            let gates = (0..self.num_reps)
                .flat_map(|i| {
                    (0..r1i_size)
                        .map(move |j| relay_mul_const((i, j), F::from_str_vartime(QIS[i]).unwrap()))
                })
                .collect_vec();

            circuit.insert(VanillaNode::new(self.num_reps, log2_size, gates.clone(), 1))
        };

        r1is.iter()
            .take(self.num_reps)
            .for_each(|&r1i| circuit.connect(r1i, r1iqis));

        let r2is = circuit.insert(InputNode::new(poly_log2_size, self.num_reps));

        let lasso_inputs_batched = {
            let r2i_log2_size = self.log2_size_with_num_reps(poly_log2_size);
            let r1i_size = 1usize << log2_size;
            let gates = chain![
                R1_BOUNDS.iter().enumerate().flat_map(|(i, &bound)| {
                    (0..r1i_size).map(move |j| relay_add_const((i, j), F::from(bound)))
                }),
                (0..(1usize << r2i_log2_size))
                    .map(move |j| relay_add_const((2, j), F::from(R2_BOUND))),
                (0..(1usize << log2_size)).map(move |j| relay_add_const((3, j), F::from(S_BOUND))),
                (0..(1usize << log2_size)).map(move |j| relay_add_const((4, j), F::from(E_BOUND))),
                (0..(1usize << log2_size)).map(move |j| relay_add_const((5, j), F::from(K1_BOUND))),
                (0..(1usize << log2_size)).map(move |j| VanillaGate::constant(F::ZERO)),
            ]
            .collect_vec();

            circuit.insert(VanillaNode::new(6, log2_size, gates, 1))
        };
        let r2is_range = {
            let r1is_log2_size = log2_size; // self.log2_size_with_num_reps(log2_size);
            let r2is_log2_size = self.log2_size_with_num_reps(poly_log2_size);
            let num_vars = log2_size + 3;
            circuit.insert(LassoNode::<F, E, C, M>::new(
                preprocessing,
                num_vars,
                chain![
                    R1_BOUNDS
                        .iter()
                        .flat_map(|&bound| iter::repeat(RangeLookup::id_for(bound * 2 + 1))
                            .take(1 << r1is_log2_size)),
                    iter::repeat(RangeLookup::id_for(R2_BOUND * 2 + 1)).take(1 << r2is_log2_size),
                    iter::repeat(RangeLookup::id_for(S_BOUND * 2 + 1)).take(1 << log2_size),
                    iter::repeat(RangeLookup::id_for(E_BOUND * 2 + 1)).take(1 << log2_size),
                    iter::repeat(RangeLookup::id_for(K1_BOUND * 2 + 1)).take(1 << log2_size),
                ]
                .collect_vec(),
            ))
        };
        r1is.iter()
            .take(self.num_reps)
            .for_each(|&r1i| circuit.connect(r1i, lasso_inputs_batched));

        connect!(circuit {
            lasso_inputs_batched <- r2is, s, e, k1;
            r2is_range <- lasso_inputs_batched;
        });

        let s_eval = circuit.insert(FftNode::forward(log2_size));
        circuit.connect(s, s_eval);

        let s_eval_copy = circuit.insert(VanillaNode::new(
            1,
            log2_size,
            (0..1usize << log2_size)
                .map(|i| VanillaGate::relay((0, i)))
                .collect_vec(),
            1,
        ));
        circuit.connect(s_eval, s_eval_copy);

        let sai_par = {
            let gates = (0..self.num_reps)
                .flat_map(|i| (0..(1usize << log2_size)).map(move |j| VanillaGate::relay((i, j))))
                .collect_vec();

            circuit.insert(VanillaNode::new(self.num_reps, log2_size, gates.clone(), 1))
        };

        for &ai in ais.iter().take(self.num_reps) {
            let gates = (0..1usize << log2_size)
                .map(|i| VanillaGate::mul((0, i), (1, i)))
                .collect_vec();
            let ai_eval = circuit.insert(FftNode::forward(log2_size));
            let sai_eval = circuit.insert(VanillaNode::new(2, log2_size, gates, 1));
            let sai = circuit.insert(FftNode::inverse(log2_size));

            connect!(circuit {
                ai_eval <- ai;
                sai_eval <- s_eval_copy, ai_eval;
                sai <- sai_eval;
            });

            circuit.connect(sai, sai_par);
        }

        let r2i_cyclo = {
            let r2i_size = (1usize << poly_log2_size) - 1;
            let gates = chain![
                (0..r2i_size).map(|i| VanillaGate::relay((0, i))),
                [VanillaGate::constant(F::ZERO)],
                (0..r2i_size).map(|i| VanillaGate::relay((0, i))),
                [VanillaGate::constant(F::ZERO)]
            ]
            .collect_vec();

            circuit.insert(VanillaNode::new(
                1,
                poly_log2_size,
                gates.clone(),
                self.num_reps,
            ))
        };

        let sum = {
            let gates = (0..1usize << log2_size)
                .map(|i| VanillaGate::sum(vec![(0, i), (1, i), (2, i), (3, i), (4, i)]))
                .collect();
            circuit.insert(VanillaNode::new(5, log2_size, gates, self.num_reps))
        };

        connect!(circuit {
            // r2is_range <- r2is_m, r2is_t, r2is;
            r2i_cyclo <- r2is;
            sum <- sai_par, es, k1kis, r1iqis, r2i_cyclo;
        });

        k1
    }

    fn log2_size_with_num_reps(&self, poly_log2_size: usize) -> usize {
        poly_log2_size + self.num_reps - 1
    }
}

pub struct BfvEncrypt<const POLY_LOG2_SIZE: usize> {
    block: BfvEncryptBlock<POLY_LOG2_SIZE>,
}

impl<const POLY_LOG2_SIZE: usize> BfvEncrypt<POLY_LOG2_SIZE> {
    pub fn new(num_reps: usize) -> Self {
        Self {
            block: BfvEncryptBlock { num_reps },
        }
    }

    pub const fn log2_size(&self) -> usize {
        // self.perm.log2_size()
        POLY_LOG2_SIZE + 1
    }

    #[allow(clippy::type_complexity)]
    pub fn setup<
        F: PrimeField,
        E: ExtensionField<F>,
        Pcs: PolynomialCommitmentScheme<F, Polynomial = MultilinearPolynomial<F>>,
    >(
        &self,
        rng: impl RngCore + Clone,
    ) -> (ProverKey<F, E, Pcs>, VerifierKey<F, E, Pcs>) {
        let lasso_preprocessing = LassoPreprocessing::<F, E>::preprocess::<C, M>(chain![
            [
                RangeLookup::new_boxed(S_BOUND * 2 + 1),
                RangeLookup::new_boxed(E_BOUND * 2 + 1),
                RangeLookup::new_boxed(K1_BOUND * 2 + 1),
                RangeLookup::new_boxed(R2_BOUND * 2 + 1),
            ],
            R1_BOUNDS
                .iter()
                .map(|&bound| RangeLookup::new_boxed(bound * 2 + 1))
        ]);

        let (comms, pks, vks): (
            Vec<Pcs::Commitment>,
            Vec<Pcs::ProverParam>,
            Vec<Pcs::VerifierParam>,
        ) = info_span!("pcs commit tables").in_scope(|| {
            iter::once({
                let t = vec![F::ZERO, F::ONE, F::ZERO - F::ONE, F::ZERO];
                let p = Pcs::setup(t.len(), 1, rng.clone()).unwrap();
                let (pp, vp) = Pcs::trim(&p, t.len(), 0).unwrap();

                (
                    Pcs::commit(&pp, &MultilinearPolynomial::new(t)).unwrap(),
                    pp,
                    vp,
                )
            })
            .chain(iter::once({
                let mut t = (0..=E_BOUND)
                    .flat_map(|b| [F::ZERO - F::from(b), F::from(b)])
                    .collect_vec();
                t.resize(1 << E_BOUND_LEN, F::ZERO);
                let p = Pcs::setup(t.len(), 1, rng.clone()).unwrap();
                let (pp, vp) = Pcs::trim(&p, t.len(), 0).unwrap();
                (
                    Pcs::commit(&pp, &MultilinearPolynomial::new(t)).unwrap(),
                    pp,
                    vp,
                )
            }))
            .chain(iter::once({
                let mut t = (0..=K1_BOUND)
                    .flat_map(|b| [F::ZERO - F::from(b), F::from(b)])
                    .collect_vec();
                t.resize(1 << K1_BOUND_LEN, F::ZERO);
                let p = Pcs::setup(t.len(), 1, rng.clone()).unwrap();
                let (pp, vp) = Pcs::trim(&p, t.len(), 0).unwrap();
                (
                    Pcs::commit(&pp, &MultilinearPolynomial::new(t)).unwrap(),
                    pp,
                    vp,
                )
            }))
            .chain(
                izip!(R1_BOUNDS, self.block.r1i_bound_log2_size())
                    .map(|(bound, bound_len)| {
                        let mut t = (0..=bound)
                            .flat_map(|b| [F::ZERO - F::from(b), F::from(b)])
                            .collect_vec();
                        t.resize(1 << bound_len, F::ZERO);
                        let p = Pcs::setup(t.len(), 1, rng.clone()).unwrap();
                        let (pp, vp) = Pcs::trim(&p, t.len(), 0).unwrap();
                        (
                            Pcs::commit(&pp, &MultilinearPolynomial::new(t)).unwrap(),
                            pp,
                            vp,
                        )
                    })
                    .take(self.block.num_reps),
            )
            .multiunzip()
        });

        let lasso_verifier = lasso_preprocessing.to_verifier_preprocessing();

        let pk = (lasso_preprocessing, izip!(comms.clone(), pks).collect_vec());
        let vk = (lasso_verifier, izip!(comms, vks).collect_vec());

        (pk, vk)
    }

    pub fn configure<F: PrimeField, E: ExtensionField<F>>(
        &self,
        circuit: &mut Circuit<F, E>,
        preprocessing: LassoPreprocessing<F, E>,
    ) -> NodeId {
        let log2_size = self.log2_size();

        let s = circuit.insert(InputNode::new(log2_size, 1));
        let e = circuit.insert(InputNode::new(log2_size, 1));
        let k1 = circuit.insert(InputNode::new(log2_size, 1));

        let s_m = circuit.insert(InputNode::new(2, 1));
        let s_t = circuit.insert(InputNode::new(2, 1));
        let s_range = circuit.insert(LogUpNode::new(2, log2_size, 1));

        let e_m = circuit.insert(InputNode::new(E_BOUND_LEN, 1));
        let e_t = circuit.insert(InputNode::new(E_BOUND_LEN, 1));
        let e_range = circuit.insert(LogUpNode::new(E_BOUND_LEN, log2_size, 1));

        let k1_m = circuit.insert(InputNode::new(K1_BOUND_LEN, 1));
        let k1_t = circuit.insert(InputNode::new(K1_BOUND_LEN, 1));
        let k1_range = circuit.insert(LogUpNode::new(K1_BOUND_LEN, log2_size, 1));

        connect!(circuit {
            s_range <- s_m, s_t, s;
            e_range <- e_m, e_t, e;
            k1_range <- k1_m, k1_t, k1;
        });

        self.block.configure(circuit, s, e, k1, preprocessing)
    }

    pub fn get_inputs<F: PrimeField, E: ExtensionField<F>>(
        &self,
        args: &BfvSkEncryptArgs,
    ) -> (
        Vec<BoxMultilinearPoly<'static, F, E>>,
        BoxMultilinearPoly<'static, F, E>,
    )
    where
        F::Repr: Into<u64>,
    {
        let log2_size = self.log2_size();

        let s = Poly::<F>::new_padded(args.s.clone(), log2_size);
        let e = Poly::<F>::new_shifted(args.e.clone(), (1 << log2_size) - 1);
        let k1 = Poly::<F>::new_shifted(args.k1.clone(), (1 << log2_size) - 1);

        let mut r2is = vec![];
        let mut r1is = vec![];
        let mut ais = vec![];
        let mut ct0is = vec![];

        for z in 0..min(args.ct0is.len(), self.block.num_reps) {
            let r2i = Poly::<F>::new(args.r2is[z].clone());
            r2is.push(r2i.to_vec());

            let r1i = Poly::<F>::new_padded(args.r1is[z].clone(), log2_size);
            r1is.push(r1i.to_vec());

            let ai = Poly::<F>::new_padded(args.ais[z].clone(), log2_size);
            ais.push(ai.to_vec());

            let ct0i = Poly::<F>::new_shifted(args.ct0is[z].clone(), 1 << log2_size);
            let mut ct0i = ct0i.as_ref()[1..].to_vec();
            ct0i.push(F::ZERO);
            ct0is.extend(ct0i);
        }

        let r2is = r2is
            .into_iter()
            .take(self.block.num_reps)
            .flat_map(|mut r2i| {
                r2i.push(F::ZERO);
                r2i
            })
            .collect_vec();

        let (s_t, s_m) = {
            let t = [F::ZERO, F::ONE, F::ZERO - F::ONE, F::ZERO];
            let mut m = [F::ZERO; 4];
            s.to_vec().iter().for_each(|s| {
                if let Some(i) = t.iter().position(|e| e == s) {
                    m[i] += F::ONE;
                }
            });

            (t.to_vec(), m.to_vec())
        };

        let (e_t, e_m) = {
            let mut t = (0..=E_BOUND)
                .flat_map(|b| [F::ZERO - F::from(b), F::from(b)])
                .collect_vec();
            t.resize(1 << E_BOUND_LEN, F::ZERO);
            let mut m = vec![F::ZERO; 1 << E_BOUND_LEN];
            e.as_ref().iter().for_each(|s| {
                if let Some(i) = t.iter().position(|e| e == s) {
                    m[i] += F::ONE;
                }
            });

            (t, m)
        };

        let (k1_t, k1_m) = {
            let mut t = (0..=K1_BOUND)
                .flat_map(|b| [F::ZERO - F::from(b), F::from(b)])
                .collect_vec();
            t.resize(1 << K1_BOUND_LEN, F::ZERO);
            let mut m = vec![F::ZERO; 1 << K1_BOUND_LEN];
            k1.as_ref().iter().for_each(|s| {
                if let Some(i) = t.iter().position(|e| e == s) {
                    m[i] += F::ONE;
                }
            });

            (t, m)
        };

        let inputs = chain_par![
            [s.to_vec(), e.to_vec(), k1.to_vec()],
            [s_m, s_t],
            [e_m, e_t],
            [k1_m, k1_t],
            ais,
            r1is,
            [r2is],
        ]
        .map(box_dense_poly)
        .collect();

        let output = box_dense_poly(ct0is);

        (inputs, output)
    }

    pub fn prove<
        F: PrimeField,
        Pcs: PolynomialCommitmentScheme<
            F,
            Polynomial = MultilinearPolynomial<F>,
            CommitmentChunk = Output<Keccak256>,
        >,
    >(
        &self,
        args: &BfvSkEncryptArgs,
        pk: ProverKey<F, F, Pcs>,
    ) -> Vec<u8>
    where
        F::Repr: Into<u64>,
    {
        let (preprocessing, pk) = pk;
        let mut transcript = Keccak256Transcript::<Vec<u8>>::default();

        let circuit = {
            let mut circuit = Circuit::<F, F>::default();
            self.configure(&mut circuit, preprocessing);
            circuit
        };
        let (values, output_claims) = info_span!("wintess gen").in_scope(|| {
            let (inputs, ctis_poly) =
                info_span!("parse inputs").in_scope(|| self.get_inputs::<F, F>(args));

            let values = info_span!("eval circuit").in_scope(|| circuit.evaluate(inputs));

            // let values = info_span!("gen values").in_scope(|| self.gen_values::<F, F>(args));

            let ct0is_claim = info_span!("eval output").in_scope(|| {
                let point = transcript.squeeze_challenges(self.ct0is_log2_size());
                let value = ctis_poly.evaluate(&point);
                EvalClaim::new(point.clone(), value)
            });

            let mut output_claims = vec![EvalClaim::new(vec![], F::ZERO); 4]; // 4 + self.block.num_reps// should be self.block.num_reps * 2 (for r2is range checks)
            output_claims.push(ct0is_claim);

            (values, output_claims)
        });

        let claims = info_span!("GKR prove")
            .in_scope(|| gkr::prove_gkr(&circuit, &values, &output_claims, &mut transcript))
            .unwrap();

        // info_span!("LogUp IOP prove").in_scope(|| {
        //     let inputs_map = circuit.inputs().collect_vec();

        //     Pcs::open(
        //         &pk[0].1,
        //         &MultilinearPolynomial::new(values[inputs_map[4]].to_dense()),
        //         &pk[0].0,
        //         &claims[4][0].point().to_vec(),
        //         &claims[4][0].value(),
        //         &mut transcript,
        //     )
        //     .expect("failed to open `s` lookup table commitment");

        //     Pcs::open(
        //         &pk[1].1,
        //         &MultilinearPolynomial::new(values[inputs_map[6]].to_dense()),
        //         &pk[1].0,
        //         &claims[6][0].point().to_vec(),
        //         &claims[6][0].value(),
        //         &mut transcript,
        //     )
        //     .expect("failed to open `e` lookup table commitment");

        //     Pcs::open(
        //         &pk[2].1,
        //         &MultilinearPolynomial::new(values[inputs_map[8]].to_dense()),
        //         &pk[2].0,
        //         &claims[8][0].point().to_vec(),
        //         &claims[8][0].value(),
        //         &mut transcript,
        //     )
        //     .expect("failed to open `k1` lookup table commitment");

        //     (0..self.block.num_reps).for_each(|i| {
        //         let t_idx = 14 + 2 * i;
        //         Pcs::open(
        //             &pk[3 + i].1,
        //             &MultilinearPolynomial::new(values[inputs_map[t_idx]].to_dense()),
        //             &pk[3 + i].0,
        //             &claims[t_idx][0].point().to_vec(),
        //             &claims[t_idx][0].value(),
        //             &mut transcript,
        //         )
        //         .expect("failed to open `r1i` lookup table commitment");
        //     });
        // });

        // let values = circuit.evaluate(expected_values);
        // assert_polys_eq(&values, &expected_values);

        transcript.into_proof()
    }

    pub fn verify<
        F: PrimeField,
        Pcs: PolynomialCommitmentScheme<
            F,
            Polynomial = MultilinearPolynomial<F>,
            CommitmentChunk = Output<Keccak256>,
        >,
    >(
        &self,
        vk: VerifierKey<F, F, Pcs>,
        proof: &[u8],
        ct0is: Vec<Vec<String>>,
    ) {
        let (preprocessing, vk) = vk;
        let mut transcript = Keccak256Transcript::from_proof(proof);

        let ct0is_claim = {
            let point = transcript.squeeze_challenges(self.ct0is_log2_size());
            let ct0is = box_dense_poly(
                ct0is
                    .into_iter()
                    .take(self.block.num_reps)
                    .flat_map(|ct0i| {
                        let ct0i = Poly::<F>::new_shifted(ct0i, 1 << self.log2_size());
                        let mut ct0i = ct0i.as_ref()[1..].to_vec();
                        ct0i.push(F::ZERO);
                        ct0i
                    })
                    .collect_vec(),
            );
            let value = ct0is.evaluate(&point);

            EvalClaim::new(point, value)
        };

        let mut output_claims = vec![EvalClaim::new(vec![], F::ZERO); 4]; // 4 + self.block.num_reps // should be self.block.num_reps * 2 (for r2is range checks)
        output_claims.push(ct0is_claim);
        // let output_claims = vec![ct0is_claim];

        let circuit = {
            let mut circuit = Circuit::<F, F>::default();
            self.configure(&mut circuit, preprocessing);
            circuit
        };

        let input_claims = info_span!("GKR verify")
            .in_scope(|| verify_gkr(&circuit, &output_claims, &mut transcript).unwrap());

        // info_span!("LogUp IOP verify").in_scope(|| {
        //     Pcs::verify(
        //         &vk[0].1,
        //         &vk[0].0,
        //         &input_claims[4][0].point().to_vec(),
        //         &input_claims[4][0].value(),
        //         &mut transcript,
        //     )
        //     .expect("failed to verify `s` lookup table commitment");

        //     Pcs::verify(
        //         &vk[1].1,
        //         &vk[1].0,
        //         &input_claims[6][0].point().to_vec(),
        //         &input_claims[6][0].value(),
        //         &mut transcript,
        //     )
        //     .expect("failed to verify `e` lookup table commitment");

        //     Pcs::verify(
        //         &vk[2].1,
        //         &vk[2].0,
        //         &input_claims[8][0].point().to_vec(),
        //         &input_claims[8][0].value(),
        //         &mut transcript,
        //     )
        //     .expect("failed to verify `k1` lookup table commitment");

        //     (0..self.block.num_reps).for_each(|i| {
        //         let t_idx = 14 + 2 * i;

        //         Pcs::verify(
        //             &vk[3 + i].1,
        //             &vk[3 + i].0,
        //             &input_claims[t_idx][0].point().to_vec(),
        //             &input_claims[t_idx][0].value(),
        //             &mut transcript,
        //         )
        //         .expect("failed to verify `r1i` lookup table commitment");
        //     });
        // });

        // izip_eq!(circuit.inputs(), input_claims).for_each(|(input, claims)| {
        //     claims
        //         .iter()
        //         .for_each(|claim| assert_eq!(values[input].evaluate(claim.point()), claim.value()))
        // });
    }

    fn ct0is_log2_size(&self) -> usize {
        assert!(self.block.num_reps.is_power_of_two());
        self.log2_size() + self.block.num_reps.next_power_of_two().ilog2() as usize
    }
}

fn root_of_unity<F: PrimeField>(k: usize) -> F {
    assert!(k <= F::S as usize);
    squares(F::ROOT_OF_UNITY).nth(F::S as usize - k).unwrap()
}

pub fn radix2_ifft<F: PrimeField>(buf: &mut [F], omega: F) {
    let n = buf.len();
    let omega_inv = omega.invert().unwrap();

    radix2_fft(buf, omega_inv);

    // Normalize the result by dividing by n
    let n_inv = F::from(n as u64).invert().unwrap(); // Assuming `Field` has an inverse method
    buf.iter_mut().for_each(|x| *x *= &n_inv);
}

fn relay_mul_const<F>(w: (usize, usize), c: F) -> VanillaGate<F> {
    VanillaGate::new(None, vec![(Some(c), w)], Vec::new())
}

fn relay_add_const<F>(w: (usize, usize), c: F) -> VanillaGate<F> {
    VanillaGate::new(Some(c), vec![(None, w)], Vec::new())
}

#[cfg(test)]
mod test {
    use super::*;
    use gkr::util::dev::seeded_std_rng;
    use goldilocks::Goldilocks;

    use std::{fs::File, io::Read};
    use tracing::{info_span, level_filters::LevelFilter};
    use tracing_forest::ForestLayer;
    use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter, Registry};

    // type Field = bn256::Fr;
    type Field = Goldilocks;

    #[test]
    pub fn test_sk_enc_valid() {
        let env_filter = EnvFilter::builder()
            .with_default_directive(LevelFilter::INFO.into())
            .from_env_lossy();

        Registry::default()
            .with(env_filter)
            .with(ForestLayer::default())
            .init();

        let rng = seeded_std_rng();

        // println!(
        //     "r2i_bound_log2_size {}",
        //     BfvEncryptBlock::r2i_bound_log2_size()
        // );

        let file_path = "src/data/sk_enc_1024_2x55_65537.json";
        let mut file = File::open(file_path).unwrap();
        let mut data = String::new();
        file.read_to_string(&mut data).unwrap();
        let bfv = BfvEncrypt::<10>::new(2);
        let args = serde_json::from_str::<BfvSkEncryptArgs>(&data).unwrap();

        let (pk, vk) = info_span!("setup")
            .in_scope(|| bfv.setup::<Field, Field, Brakedown<Field>>(rng.clone()));
        let proof = info_span!("FHE_enc prove")
            .in_scope(|| bfv.prove::<Field, Brakedown<Field>>(&args, pk));

        info_span!("FHE_enc verify")
            .in_scope(|| bfv.verify::<Field, Brakedown<Field>>(vk, &proof, args.ct0is));
    }
}

// pub fn gen_values<F: PrimeField, E: ExtensionField<F>>(
//     &self,
//     args: &BfvSkEncryptArgs,
// ) -> Vec<BoxMultilinearPoly<'static, F, E>>
// where
//     F::Repr: Into<u64>,
// {
//     let log2_size = self.log2_size();

//     let s = Poly::<F>::new_padded(args.s.clone(), log2_size);
//     let e = Poly::<F>::new_shifted(args.e.clone(), (1 << log2_size) - 1);
//     let k1 = Poly::<F>::new_shifted(args.k1.clone(), (1 << log2_size) - 1);

//     let mut r2is = vec![];
//     let mut r1is = vec![];
//     let mut ais = vec![];
//     let mut ct0is = vec![];

//     let mut qi_constants = vec![];
//     let mut k0i_constants = vec![];

//     for z in 0..min(args.ct0is.len(), self.block.num_reps) {
//         let r2i = Poly::<F>::new(args.r2is[z].clone());
//         r2is.push(r2i.to_vec());

//         let r1i = Poly::<F>::new_padded(args.r1is[z].clone(), log2_size);
//         r1is.push(r1i.to_vec());

//         let ai = Poly::<F>::new_padded(args.ais[z].clone(), log2_size);
//         ais.push(ai.to_vec());

//         let ct0i = Poly::<F>::new_shifted(args.ct0is[z].clone(), 1 << log2_size);
//         let mut ct0i = ct0i.as_ref()[1..].to_vec();
//         ct0i.push(F::ZERO);
//         ct0is.extend(ct0i);

//         qi_constants.push(F::from_str_vartime(QIS[z]).unwrap());
//         k0i_constants.push(F::from_str_vartime(K0IS[z]).unwrap());
//     }

//     let es = (0..self.block.num_reps)
//         .flat_map(|_| e.as_ref().to_vec())
//         .collect_vec();
//     let k1k0is = (0..self.block.num_reps)
//         .flat_map(|i| {
//             k1.as_ref()
//                 .iter()
//                 .map(move |&k1| k1 * F::from_str_vartime(K0IS[i]).unwrap())
//         })
//         .collect_vec();
//     let r1iqis = (0..self.block.num_reps)
//         .flat_map(|i| {
//             r1is[i]
//                 .iter()
//                 .map(move |&r1i| r1i * F::from_str_vartime(QIS[i]).unwrap())
//         })
//         .collect_vec();

//     let omega = root_of_unity(log2_size);

//     let s_eval = {
//         let mut buf = s.to_vec();
//         radix2_fft(&mut buf, omega);
//         buf
//     };

//     let ai_evals = ais
//         .iter()
//         .map(|ai| {
//             let mut buf = ai.clone();
//             radix2_fft(&mut buf, omega);
//             buf
//         })
//         .collect_vec();

//     let sai_evals = ai_evals
//         .iter()
//         .map(|ai_eval| {
//             izip!(s_eval.clone(), ai_eval.clone())
//                 .map(|(s_e, ai_e)| s_e * ai_e)
//                 .collect_vec()
//         })
//         .collect_vec();

//     let sais: Vec<_> = sai_evals
//         .par_iter()
//         .map(|sai_eval| {
//             let mut buf = sai_eval.clone();
//             radix2_ifft(&mut buf, omega);
//             buf
//         })
//         .collect();

//     let sai_values = izip!(ai_evals, sai_evals, sais.clone())
//         .flat_map(|(ai_eval, sai_eval, sai)| [ai_eval, sai_eval, sai])
//         .collect_vec();

//     let sai = sais.iter().flatten().cloned().collect_vec();

//     let r2is_cyclo = r2is
//         .iter()
//         .take(self.block.num_reps)
//         .flat_map(|r2i| {
//             let mut result = vec![F::ZERO; 2 * N]; // Allocate result vector of size 2N-1

//             for i in 0..r2i.len() {
//                 result[i] += r2i[i]; // Add P(x)
//                 result[i + N] += r2i[i]; // Add P(x) * x^N
//             }
//             result
//         })
//         .collect_vec();

//     let r2is = r2is
//         .into_iter()
//         .take(self.block.num_reps)
//         .flat_map(|mut r2i| {
//             r2i.push(F::ZERO);
//             r2i
//         })
//         .collect_vec();

//     let r2is_shifted = r2is
//         .iter()
//         .map(|&r2i| r2i + F::from(R2_BOUND))
//         .collect_vec();

//     let (s_t, s_m) = {
//         let t = [F::ZERO, F::ONE, F::ZERO - F::ONE, F::ZERO];
//         let mut m = [F::ZERO; 4];
//         s.to_vec().iter().for_each(|s| {
//             if let Some(i) = t.iter().position(|e| e == s) {
//                 m[i] += F::ONE;
//             }
//         });

//         (t.to_vec(), m.to_vec())
//     };

//     let (e_t, e_m) = {
//         let mut t = (0..=E_BOUND)
//             .flat_map(|b| [F::ZERO - F::from(b), F::from(b)])
//             .collect_vec();
//         t.resize(1 << E_BOUND_LEN, F::ZERO);
//         let mut m = vec![F::ZERO; 1 << E_BOUND_LEN];
//         e.as_ref().iter().for_each(|s| {
//             if let Some(i) = t.iter().position(|e| e == s) {
//                 m[i] += F::ONE;
//             }
//         });

//         (t, m)
//     };

//     let (k1_t, k1_m) = {
//         let mut t = (0..=K1_BOUND)
//             .flat_map(|b| [F::ZERO - F::from(b), F::from(b)])
//             .collect_vec();
//         t.resize(1 << K1_BOUND_LEN, F::ZERO);
//         let mut m = vec![F::ZERO; 1 << K1_BOUND_LEN];
//         k1.as_ref().iter().for_each(|s| {
//             if let Some(i) = t.iter().position(|e| e == s) {
//                 m[i] += F::ONE;
//             }
//         });

//         (t, m)
//     };

//     let r1i_range_values = izip!(R1_BOUNDS, self.block.r1i_bound_log2_size(), &r1is)
//         .map(|(bound, bound_len, r1i)| {
//             let mut t = (0..=bound)
//                 .flat_map(|b| [F::ZERO - F::from(b), F::from(b)])
//                 .collect_vec();
//             t.resize(1 << bound_len, F::ZERO);
//             let mut m = vec![F::ZERO; 1 << bound_len];
//             r1i.iter().for_each(|s| {
//                 if let Some(i) = t.iter().position(|e| e == s) {
//                     m[i] += F::ONE;
//                 }
//             });

//             (m, t)
//         })
//         .take(self.block.num_reps)
//         .flat_map(|(m, t)| [m, t, vec![F::ZERO]])
//         .collect_vec();

//     chain_par![
//         [s.to_vec(), e.to_vec(), k1.to_vec()],
//         [s_m, s_t, vec![F::ZERO]],   // s_range
//         [e_m, e_t, vec![F::ZERO]],   // e_range
//         [k1_m, k1_t, vec![F::ZERO]], // k1_range
//         [es, k1k0is],
//         ais,
//         r1is,
//         r1i_range_values, // r1i_range
//         [r1iqis],
//         [r2is, r2is_shifted, vec![F::ZERO]],
//         // [r2is],
//         // [r2is_m, r2is_t, vec![F::ZERO]] // r2is_range
//         [s_eval.clone()],
//         [s_eval],
//         [sai],
//         sai_values,
//         [r2is_cyclo],
//         [ct0is]
//     ]
//     .map(box_dense_poly)
//     .collect()
// }
