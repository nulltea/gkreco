mod poly;

use bfv::{generate_prime, Modulus, NttOperator, Poly, Representation};
use bfv::{
    BfvParameters, Ciphertext, Encoding, EncodingType, Plaintext, PolyCache, PublicKey, SecretKey,
};
use concrete_ntt::native64::Plan32;
use fhe_traits::*;
use itertools::{izip, Itertools};
use num_bigint::{BigInt, BigUint};
use num_traits::{FromPrimitive, Num, One, Signed, ToPrimitive, Zero};
use rand::rngs::StdRng;
use rand::SeedableRng;
use rayon::iter::{ParallelBridge, ParallelIterator};
use serde_json::json;
use std::io::Write;
use std::ops::Deref;
use std::path::Path;
use std::sync::Arc;
use std::vec;
use std::{collections::HashMap, fs::File};
use tracing::info_span;
use tracing_forest::ForestLayer;
use tracing_subscriber::{layer::SubscriberExt, EnvFilter, Registry};

use poly::*;

/// Set of vectors for input validation of a ciphertext
#[derive(Clone, Debug)]
pub struct InputValidationVectors {
    ct0is: Vec<Vec<BigInt>>,
    ais: Vec<Vec<BigInt>>,
    r1is: Vec<Vec<BigInt>>,
    r2is: Vec<Vec<BigInt>>,
    k0is: Vec<BigInt>,
    e: Vec<BigInt>,
    s: Vec<BigInt>,
    k1: Vec<BigInt>,
}

/// The `InputValidationBounds` struct holds the bounds for various vectors and polynomials used in the input validation process.
/// These bounds are calculated from a set of BFV encryption parameters and represent limits on the values of different fields
/// to ensure that the inputs remain within valid ranges during operations.
#[derive(Clone, Debug)]
pub struct InputValidationBounds {
    u: BigInt,
    e: BigInt,
    t: BigInt,
    k1: BigInt,
    r1: Vec<BigInt>,
    r2: Vec<BigInt>,
}

impl InputValidationBounds {
    // pub fn check_constraints(&self, vecs: &InputValidationVectors, p: &BigInt) {
    //     let vecs_std = vecs.standard_form(p);

    //     // constraint. The coefficients of u, e0, e1 should be in the range [-⌈6σ⌋, ⌈6σ⌋]
    //     // where ⌈6σ⌋ is the upper bound of the discrete Gaussian distribution
    //     assert!(range_check_centered(&vecs.u, &-&self.u, &self.u));
    //     assert!(range_check_centered(&vecs.e0, &-&self.e, &self.e));
    //     assert!(range_check_centered(&vecs.e1, &-&self.e, &self.e));
    //     assert!(range_check_standard(&vecs_std.u, &self.u, &p));
    //     assert!(range_check_standard(&vecs_std.e0, &self.e, &p));
    //     assert!(range_check_standard(&vecs_std.e1, &self.e, &p));

    //     // constraint. The coefficients of k1 should be in the range [-(t-1)/2, (t-1)/2]
    //     assert!(range_check_centered(&vecs.k1, &-&self.k1, &self.k1));
    //     assert!(range_check_standard(&vecs_std.k1, &self.k1, &p));

    //     // Perform asserts for polynomials depending on each qi
    //     for i in 0..self.r2.len() {
    //         // constraint. The coefficients of ct0i and ct1i should be in the range [-(qi-1)/2, (qi-1)/2]
    //         assert!(range_check_centered(
    //             &vecs.ct0is[i],
    //             &-&self.pk[i],
    //             &self.pk[i]
    //         ));
    //         assert!(range_check_centered(
    //             &vecs.ct1is[i],
    //             &-&self.pk[i],
    //             &self.pk[i]
    //         ));

    //         // constraint. The coefficients of pk0i and pk1i should be in range [-(qi-1)/2 , (qi-1)/2]
    //         assert!(range_check_centered(
    //             &vecs.pk0is[i],
    //             &-&self.pk[i],
    //             &self.pk[i]
    //         ));
    //         assert!(range_check_centered(
    //             &vecs.pk1is[i],
    //             &-&self.pk[i],
    //             &self.pk[i]
    //         ));
    //         assert!(range_check_standard(&vecs_std.pk0is[i], &self.pk[i], &p));
    //         assert!(range_check_standard(&vecs_std.pk1is[i], &self.pk[i], &p));

    //         // constraint. The coefficients of r2i should be in the range [-(qi-1)/2, (qi-1)/2]
    //         assert!(range_check_centered(
    //             &vecs.r2is[i],
    //             &-&self.r2[i],
    //             &self.r2[i]
    //         ));
    //         assert!(range_check_standard(&vecs_std.r2is[i], &self.r2[i], &p));

    //         // constraint. The coefficients of (ct0i - ct0i_hat - r2i * cyclo) / qi = r1i should be in the range
    //         // $[
    //         //      \frac{- ((N+2) \cdot \frac{q_i - 1}{2} + B + \frac{t - 1}{2} \cdot |K_{0,i}|)}{q_i},
    //         //      \frac{   (N+2) \cdot \frac{q_i - 1}{2} + B + \frac{t - 1}{2} \cdot |K_{0,i}| }{q_i}
    //         // ]$
    //         assert!(range_check_centered(
    //             &vecs.r1is[i],
    //             &-&self.r1[i],
    //             &self.r1[i]
    //         ));
    //         assert!(range_check_standard(&vecs_std.r1is[i], &self.r1[i], &p));

    //         // constraint. The coefficients of p2 should be in the range [-(qi-1)/2, (qi-1)/2]
    //         assert!(range_check_centered(
    //             &vecs.p2is[i],
    //             &-&self.p2[i],
    //             &self.p2[i]
    //         ));
    //         assert!(range_check_standard(&vecs_std.p2is[i], &self.p2[i], &p));

    //         // constraint. The coefficients of (ct0i - ct0i_hat - p2i * cyclo) / qi = p1i should be in the range
    //         // $[
    //         //      \frac{- ((N+2) \cdot \frac{q_i - 1}{2} + B)}{q_i},
    //         //      \frac{   (N+2) \cdot \frac{q_i - 1}{2} + B }{q_i}
    //         // ]$
    //         assert!(range_check_centered(
    //             &vecs.p1is[i],
    //             &-&self.p1[i],
    //             &self.p1[i]
    //         ));
    //         assert!(range_check_standard(&vecs_std.p1is[i], &self.p1[i], &p));
    //     }
    // }

    /// Compute the input validation bounds from a set of BFV encryption parameters.
    ///
    /// # Arguments
    ///
    /// * `params` - A reference to the BFV parameters.
    /// * `level` - The encryption level, which determines the number of moduli used.
    ///
    /// # Returns
    ///
    /// A new `InputValidationBounds` instance containing the bounds for vectors and polynomials
    /// based on the BFV parameters and the specified level.
    pub fn compute(
        params: &BfvParameters,
        level: usize,
    ) -> Result<InputValidationBounds, Box<dyn std::error::Error>> {
        // Get cyclotomic degree and context at provided level
        let N = BigInt::from(params.degree);
        let t = BigInt::from(params.plaintext_modulus);
        let ctx = params.poly_ctx(&bfv::PolyType::Q, level);

        // Note: the secret key in fhe.rs is sampled from a discrete gaussian distribution
        // rather than a ternary distribution as in bfv.py.
        let gauss_bound = BigInt::from(
            f64::ceil(6_f64 * f64::sqrt(params.variance as f64))
                .to_i64()
                .ok_or_else(|| "Failed to convert variance to i64".to_string())?,
        );
        let u_bound = gauss_bound.clone();
        let e_bound = gauss_bound.clone();
        let ptxt_bound = (t - BigInt::from(1)) / BigInt::from(2);
        let k1_bound = ptxt_bound.clone();

        // Calculate qi-based bounds
        let num_moduli = ctx.moduli_count();
        let mut r2_bounds: Vec<BigInt> = vec![BigInt::zero(); num_moduli];
        let mut r1_bounds: Vec<BigInt> = vec![BigInt::zero(); num_moduli];
        for (i, qi) in ctx.moduli_ops().iter().enumerate() {
            let qi_bigint = BigInt::from(qi.modulus());
            let qi_bound = (&qi_bigint - BigInt::from(1)) / BigInt::from(2);

            // Calculate the k0qi for the bounds (these are also constant wrt BFV params)
            let k0qi = BigInt::from(
                qi.inv(qi.neg_mod_fast(params.plaintext_modulus))
            );

            // pk_bounds[i] = qi_bound.clone();
            r2_bounds[i] = qi_bound.clone();
            r1_bounds[i] = ((&N + 2) * &qi_bound + &gauss_bound + &ptxt_bound * BigInt::abs(&k0qi))
                / &qi_bigint;
        }

        Ok(InputValidationBounds {
            u: u_bound,
            e: e_bound,
            t: ptxt_bound,
            k1: k1_bound,
            r1: r1_bounds,
            r2: r2_bounds,
        })
    }
}

impl InputValidationVectors {
    /// Create a new `InputValidationVectors` with the given number of moduli and degree.
    ///
    /// # Arguments
    ///
    /// * `num_moduli` - The number of moduli, which determines the number of inner vectors in 2D vectors.
    /// * `degree` - The size of each inner vector in the 2D vectors.
    ///
    /// # Returns
    ///
    /// Returns a new instance of `InputValidationVectors` with all fields initialized to zero.
    pub fn new(num_moduli: usize, degree: usize) -> Self {
        InputValidationVectors {
            ct0is: vec![vec![BigInt::zero(); degree]; num_moduli],
            ais: vec![vec![BigInt::zero(); degree]; num_moduli],
            r1is: vec![vec![BigInt::zero(); 2 * (degree - 1)]; num_moduli],
            r2is: vec![vec![BigInt::zero(); degree - 2]; num_moduli],
            k0is: vec![BigInt::zero(); num_moduli],
            e: vec![BigInt::zero(); degree],
            s: vec![BigInt::zero(); degree],
            k1: vec![BigInt::zero(); degree],
        }
    }

    // /// Assign and return all of the centered input validation vectors to the ZKP modulus `p`.
    // ///
    // /// # Arguments
    // ///
    // /// * `p` - ZKP modulus
    // ///
    // /// # Returns
    // ///
    // /// Returns a new `InputValidationVectors` struct with all coefficients reduced modulo `p`.
    pub fn standard_form(&self, p: &BigInt) -> Self {
        InputValidationVectors {
            ais: reduce_coefficients_2d(&self.ais, p),
            ct0is: reduce_coefficients_2d(&self.ct0is, p),
            r1is: reduce_coefficients_2d(&self.r1is, p),
            r2is: reduce_coefficients_2d(&self.r2is, p),
            k0is: self.k0is.clone(),
            e: reduce_coefficients(&self.e, p),
            s: reduce_coefficients(&self.s, p),
            k1: reduce_coefficients(&self.k1, p),
        }
    }

    /// Convert the `InputValidationVectors` to a JSON object.
    ///
    /// # Returns
    ///
    /// Returns a `serde_json::Value` representing the JSON serialization of the `InputValidationVectors`.
    // pub fn to_json(&self) -> serde_json::Value {
    //     json!({
    //         "pk0i": to_string_2d_vec(&self.pk0is),
    //         "pk1i": to_string_2d_vec(&self.pk1is),
    //         "u": to_string_1d_vec(&self.u),
    //         "e0": to_string_1d_vec(&self.e0),
    //         "e1": to_string_1d_vec(&self.e1),
    //         "k1": to_string_1d_vec(&self.k1),
    //         "r2is": to_string_2d_vec(&self.r2is),
    //         "r1is": to_string_2d_vec(&self.r1is),
    //         "p2is": to_string_2d_vec(&self.p2is),
    //         "p1is": to_string_2d_vec(&self.p1is),
    //         "ct0is": to_string_2d_vec(&self.ct0is),
    //         "ct1is": to_string_2d_vec(&self.ct1is),
    //     })
    // }

    /// Check whether all members of `self` have the correct length based on the provided `degree` and `num_moduli`.
    ///
    /// # Arguments
    ///
    /// * `num_moduli` - The expected number of moduli (outer vector length).
    /// * `degree` - The expected degree (inner vector length).
    ///
    /// # Returns
    ///
    /// Returns `true` if all vectors have the correct lengths, `false` otherwise.
    // pub fn check_correct_lengths(&self, num_moduli: usize, degree: usize) -> bool {
    //     // Helper function to check 2D vector lengths
    //     let check_2d_lengths =
    //         |vec: &Vec<Vec<BigInt>>, expected_outer_len: usize, expected_inner_len: usize| {
    //             vec.len() == expected_outer_len && vec.iter().all(|v| v.len() == expected_inner_len)
    //         };

    //     // Helper function to check 1D vector lengths
    //     let check_1d_lengths = |vec: &Vec<BigInt>, expected_len: usize| vec.len() == expected_len;

    //     // Use all to combine all checks into a single statement
    //     [
    //         // 2D vector checks
    //         check_2d_lengths(&self.pk0is, num_moduli, degree),
    //         check_2d_lengths(&self.pk1is, num_moduli, degree),
    //         check_2d_lengths(&self.ct0is, num_moduli, degree),
    //         check_2d_lengths(&self.ct1is, num_moduli, degree),
    //         check_2d_lengths(&self.r1is, num_moduli, 2 * (degree - 1)),
    //         check_2d_lengths(&self.r2is, num_moduli, degree - 2),
    //         check_2d_lengths(&self.p1is, num_moduli, 2 * (degree - 1)),
    //         check_2d_lengths(&self.p2is, num_moduli, degree - 2),
    //         // 1D vector checks
    //         check_1d_lengths(&self.k0is, num_moduli),
    //         check_1d_lengths(&self.u, degree),
    //         check_1d_lengths(&self.e0, degree),
    //         check_1d_lengths(&self.e1, degree),
    //         check_1d_lengths(&self.k1, degree),
    //     ]
    //     .iter()
    //     .all(|&check| check)
    // }

    /// Create the centered validation vectors necessary for creating an input validation proof according to Greco.
    /// For more information, please see https://eprint.iacr.org/2024/594.
    ///
    /// # Arguments
    ///
    /// * `pt` - Plaintext from fhe.rs.
    /// * `u_rns` - Private polynomial used in ciphertext sampled from secret key distribution.
    /// * `e0_rns` - Error polynomial used in ciphertext sampled from error distribution.
    /// * `e1_rns` - Error polynomioal used in cihpertext sampled from error distribution.
    /// * `ct` - Ciphertext from fhe.rs.
    /// * `pk` - Public Key from fhe.rs.
    pub fn compute(
        params: &BfvParameters,
        pt: &Plaintext,
        e_rns: &Poly,
        // e0_rns: &Poly,
        // e1_rns: &Poly,
        ct: &Ciphertext,
        sk: &SecretKey,
        // pk: &PublicKey,
    ) -> Result<InputValidationVectors, Box<dyn std::error::Error>> {
        // Get context, plaintext modulus, and degree
        let ctx = params.poly_ctx(&bfv::PolyType::Q, pt.level());
        let t = Modulus::new(params.plaintext_modulus);
        let N: u64 = ctx.degree() as u64;

        let mut product = BigUint::one();
        ctx.moduli_ops().iter().for_each(|m| {
            product *= BigUint::from_u64(m.modulus()).unwrap();
        });

        // Calculate k1 (independent of qi), center and reverse
        let q_mod_t = (product % t.modulus())
            .to_u64()
            .ok_or_else(|| "Cannot convert BigInt to u64.".to_string())?; // [q]_t
                                                                          // println!("q mod t: {:?}", q_mod_t);
        let mut k1_u64 = pt.value().deref().to_vec(); // m
        t.scalar_mul_vec(&mut k1_u64, q_mod_t); // k1 = [q*m]_t
        let mut k1: Vec<BigInt> = k1_u64.iter().map(|&x| BigInt::from(x)).rev().collect();
        reduce_and_center_coefficients_mut(&mut k1, &BigInt::from(t.modulus()));

        // println!("u start: {:?}", u_rns.coefficients());

        // Extract single vectors of u, e1, and e2 as Vec<BigInt>, center and reverse
        let mut e_rns_copy = e_rns.clone();
        // let mut e0_rns_copy = e0_rns.clone();
        // let mut e1_rns_copy = e1_rns.clone();

        // println!("e: {:?}", e_rns_copy.representation());

        ctx.change_representation(&mut e_rns_copy, Representation::Coefficient);

        //    println!("e: {:?}", e_rns_copy.coefficients());

        let e: Vec<BigInt> = unsafe {
            ctx.moduli_ops()[0]
                .center_vec_vt(
                    e_rns_copy
                        .coefficients()
                        .row(0)
                        .as_slice()
                        .ok_or_else(|| "Cannot center coefficients.".to_string())?,
                )
                .iter()
                .rev()
                .map(|&x| BigInt::from(x))
                .collect()
        };

        let mut s_poly = sk.to_poly(&ctx);
        // println!("s: {:?}", s_poly.coefficients());
        ctx.change_representation(&mut s_poly, Representation::Coefficient);

        let mut s: Vec<BigInt> = s_poly
            .coefficients()
            .row(0)
            .iter()
            .rev()
            .map(|&x| BigInt::from(x))
            .collect();

        // println!("s: {:?}", s);

        // let e1: Vec<BigInt> = unsafe {
        //     ctx.moduli_ops()[0]
        //         .center_vec_vt(
        //             e1_rns_copy
        //                 .coefficients()
        //                 .row(0)
        //                 .as_slice()
        //                 .ok_or_else(|| "Cannot center coefficients.".to_string())?,
        //         )
        //         .iter()
        //         .rev()
        //         .map(|&x| BigInt::from(x))
        //         .collect()
        // };

        // println!("ct0: {:?}", ct.c_ref()[0].coefficients());

        // Extract and convert ciphertext and plaintext polynomials
        let mut ct0 = ct.c_ref()[0].clone();
        let mut ct1 = ct.c_ref()[1].clone();
        ctx.change_representation(&mut ct0, Representation::Coefficient);
        ctx.change_representation(&mut ct1, Representation::Coefficient);

        println!("ct0: {:?}", ct0.coefficients());
        println!("ct1: {:?}", ct1.coefficients());

        // ctx.change_representation(&mut pk1, Representation::Coefficient);

        // Create cyclotomic polynomial x^N + 1
        let mut cyclo = vec![BigInt::from(0u64); (N + 1) as usize];
        cyclo[0] = BigInt::from(1u64); // x^N term
        cyclo[N as usize] = BigInt::from(1u64); // x^0 term

        // Initialize matrices to store results
        let num_moduli = ctx.moduli_count();
        let mut res = InputValidationVectors::new(num_moduli, N as usize);

        let plan = PlanNtt::try_new(N as usize * 2).unwrap();
        #[cfg(feature = "sanity-check")]
        let plan_cyclo = PlanNtt::try_new(N as usize * 4).unwrap();

        // Perform the main computation logic
        #[allow(clippy::type_complexity)]
        let results: Vec<(
            usize,
            Vec<BigInt>,
            Vec<BigInt>,
            Vec<BigInt>,
            BigInt,
            Vec<BigInt>,
            // Vec<BigInt>,
        )> = izip!(
            ctx.moduli_ops(),
            ct0.coefficients().rows(),
            ct1.coefficients().rows(),
        )
        .enumerate()
        .take(1)
        // .par_bridge()
        .map(|(i, (qi, ct0_coeffs, ct1_coeffs))| {
            // --------------------------------------------------- ct0i ---------------------------------------------------
            info_span!("results", i).in_scope(|| {
                // Convert to vectors of bigint, center, and reverse order.
                let mut ct0i: Vec<BigInt> =
                    ct0_coeffs.iter().rev().map(|&x| BigInt::from(x)).collect();
                let mut ct1i: Vec<BigInt> =
                    ct1_coeffs.iter().rev().map(|&x| BigInt::from(x)).collect();
                // let mut pk0i: Vec<BigInt> =
                //     pk0_coeffs.iter().rev().map(|&x| BigInt::from(x)).collect();
                // let mut pk1i: Vec<BigInt> =
                //     pk1_coeffs.iter().rev().map(|&x| BigInt::from(x)).collect();

                let qi_bigint = BigInt::from(qi.modulus());

                reduce_and_center_coefficients_mut(&mut ct0i, &qi_bigint);
                reduce_and_center_coefficients_mut(&mut ct1i, &qi_bigint);
                // reduce_and_center_coefficients_mut(&mut pk0i, &qi_bigint);
                // reduce_and_center_coefficients_mut(&mut pk1i, &qi_bigint);

                // k0qi = -t^{-1} mod qi
                let koqi_u64 = qi.inv(qi.neg_mod_fast(t.modulus()));
                let k0qi = BigInt::from(koqi_u64); // Do not need to center this

                // ki = k1 * k0qi
                let ki = poly_scalar_mul(&k1, &k0qi);
                // println!("ki: {:?}", ki);

                let ai = poly_scalar_mul(&ct1i, &BigInt::from_i64(-1).unwrap());
                // Calculate ct0i_hat = ai * s + e
                let ct0i_hat = info_span!("compute ct0i_hat").in_scope(|| {
                    let pk0i_times_u = poly_mul(&plan, &ai, &s);
                    // println!("u: {:?}", u);

                    #[cfg(feature = "sanity-check")]
                    assert_eq!((pk0i_times_u.len() as u64) - 1, 2 * (N - 1));

                    let e_plus_ki = poly_add(&e, &ki);

                    #[cfg(feature = "sanity-check")]
                    assert_eq!((e_plus_ki.len() as u64) - 1, N - 1);

                    poly_add(&pk0i_times_u, &e_plus_ki)
                });
                // println!("ct0i_hat: {:?}", ct0i_hat);

                #[cfg(feature = "sanity-check")]
                assert_eq!((ct0i_hat.len() as u64) - 1, 2 * (N - 1));

                // Check whether ct0i_hat mod R_qi (the ring) is equal to ct0i
                let mut ct0i_hat_mod_rqi = ct0i_hat.clone();
                info_span!("reduce_in_ring: ct0i_hat_mod_rqi % qi").in_scope(|| {
                    reduce_in_ring(&mut ct0i_hat_mod_rqi, &cyclo, &qi_bigint);
                });

                println!("ct0i_hat_mod_rqi: {:?}", ct0i_hat_mod_rqi);
                #[cfg(feature = "sanity-check")]
                assert_eq!(&ct0i, &ct0i_hat_mod_rqi);

                // Compute r2i numerator = ct0i - ct0i_hat and reduce/center the polynomial
                let ct0i_minus_ct0i_hat = poly_sub(&ct0i, &ct0i_hat);

                #[cfg(feature = "sanity-check")]
                assert_eq!((ct0i_minus_ct0i_hat.len() as u64) - 1, 2 * (N - 1));

                let mut ct0i_minus_ct0i_hat_mod_zqi = ct0i_minus_ct0i_hat.clone();
                reduce_and_center_coefficients_mut(&mut ct0i_minus_ct0i_hat_mod_zqi, &qi_bigint);

                // Compute r2i as the quotient of numerator divided by the cyclotomic polynomial
                // to produce: (ct0i - ct0i_hat) / (x^N + 1) mod Z_qi. Remainder should be empty.
                let (r2i, _r2i_rem) = info_span!("poly_div_cyclo: r2i")
                    .in_scope(|| poly_div_cyclo(&ct0i_minus_ct0i_hat_mod_zqi, cyclo.len() - 1));

                #[cfg(feature = "sanity-check")]
                {
                    assert!(_r2i_rem.is_empty());
                    assert_eq!((r2i.len() as u64) - 1, N - 2); // Order(r2i) = N - 2
                }

                // Assert that (ct0i - ct0i_hat) = (r2i * cyclo) mod Z_qi
                let r2i_times_cyclo =
                    info_span!("poly_mul: r2i * cyclo").in_scope(|| poly_mul(&plan, &r2i, &cyclo));
                let mut r2i_times_cyclo_mod_zqi = r2i_times_cyclo.clone();
                reduce_and_center_coefficients_mut(&mut r2i_times_cyclo_mod_zqi, &qi_bigint);
                #[cfg(feature = "sanity-check")]
                {
                    assert_eq!(&ct0i_minus_ct0i_hat_mod_zqi, &r2i_times_cyclo_mod_zqi);
                    assert_eq!((r2i_times_cyclo.len() as u64) - 1, 2 * (N - 1));
                }

                // Calculate r1i = (ct0i - ct0i_hat - r2i * cyclo) / qi mod Z_p. Remainder should be empty.
                let r1i_num = poly_sub(&ct0i_minus_ct0i_hat, &r2i_times_cyclo);
                #[cfg(feature = "sanity-check")]
                assert_eq!((r1i_num.len() as u64) - 1, 2 * (N - 1));

                let (r1i, _r1i_rem) = info_span!("poly_div: r1i_num / qi")
                    .in_scope(|| poly_div(&r1i_num, &[qi_bigint.clone()]));
                #[cfg(feature = "sanity-check")]
                {
                    assert!(_r1i_rem.is_empty());
                    assert_eq!((r1i.len() as u64) - 1, 2 * (N - 1)); // Order(r1i) = 2*(N-1)
                    assert_eq!(&r1i_num, &poly_mul(&plan_cyclo, &r1i, &[qi_bigint.clone()]));
                }

                // Assert that ct0i = ct0i_hat + r1i * qi + r2i * cyclo mod Z_p
                #[cfg(feature = "sanity-check")]
                {
                    let r1i_times_qi = info_span!("poly_scalar_mul: r1i * qi_bigint")
                        .in_scope(|| poly_scalar_mul(&r1i, &qi_bigint));
                    let mut ct0i_calculated =
                        poly_add(&poly_add(&ct0i_hat, &r1i_times_qi), &r2i_times_cyclo);

                    while ct0i_calculated.len() > 0 && ct0i_calculated[0].is_zero() {
                        ct0i_calculated.remove(0);
                    }

                    assert_eq!(&ct0i, &ct0i_calculated);
                }

                (i, ai, r2i, r1i, k0qi, ct0i)
            })
        })
        .collect();

        // println!("Completed creation of polynomials!");

        // Merge results into the `res` structure after parallel execution
        for (i, ai, r2i, r1i, k0i, ct0i) in results.into_iter() {
            res.ais[i] = ai;
            res.r2is[i] = r2i;
            res.r1is[i] = r1i;
            res.k0is[i] = k0i;
            res.ct0is[i] = ct0i;
        }

        // Set final result vectors
        res.e = e;
        res.s = s;
        res.k1 = k1;

        Ok(res)
    }
}

pub fn gen_witness(
    params: BfvParameters,
    ct: Ciphertext,
    e: Poly,
    // e0: Poly,
    // e1: Poly,
    pt: Plaintext,
    sk: SecretKey,
    
) -> Result<(), Box<dyn std::error::Error>> {
    // TODO: add method `default_parameter_128(plaintext_nbits: usize, log2_n: usize) -> BfvParameters` in fhe-rs fork
    // TODO: and cache?
    // let params = default_parameters_128(20, n_log2);
    let N: u64 = params.degree as u64;

    // Initialize zk proving modulus
    let p = BigInt::from_str_radix("18446744069414584321", 10)?;

    // Compute input validation vectors
    let res = InputValidationVectors::compute(&params, &pt, &e, &ct, &sk)?;

    // Create output json with standard form polynomials
    // let json_data = res.standard_form(&p).to_json();

    // Calculate bounds ---------------------------------------------------------------------
    let bounds = InputValidationBounds::compute(&params, pt.level());


    res.standard_form(&p);

    Ok(())
}

#[test]
fn test_gen_witness() {
    let mut rng = StdRng::seed_from_u64(0);

    let mut params = BfvParameters::new_with_primes(
        vec![1032193],
        vec![40961], //vec![995329, 1073668097],
        40961,
        32,
    );
    // println!("ntt: {:?}", <NttOperator as traits::Ntt>::new(32, 40961,));
    // let ctx = params.poly_ctx(&bfv::PolyType::Q, 0);

    // println!("Params: {:?}", ctx.moduli_ops());
    // params.enable_hybrid_key_switching_with_prime(vec![61441]);
    // params.enable_pke();

    let N: u64 = params.degree as u64;

    let sk = SecretKey::random_with_params(&params, &mut StdRng::seed_from_u64(0));
    // println!("sk: {:?}", sk.coefficients);
    // let pk = PublicKey::new(&params, &sk, &mut rng);
    // println!("pk c0: {:?}", pk.c0_ref().coefficients());
    // println!("pk c1: {:?}", pk.c1_ref().coefficients());

    let m: Vec<_> = (0..(N as u64)).collect_vec(); // m here is from lowest degree to largest as input into fhe.rs (REQUIRED)
    let pt = Plaintext::encode(
        &m,
        &params,
        Encoding {
            encoding_type: EncodingType::Poly,
            poly_cache: PolyCache::None,
            level: 0,
        },
    );
    // println!("pt: {:?}", pt.value());
    println!("-------------------");
    let (ct, e) = sk.encrypt(&params, &pt, &mut rng);
    println!("-------------------");

    println!("ct c0: {:?}", ct.c_ref()[0].coefficients());
    println!("ct c1: {:?}", ct.c_ref()[1].coefficients());
    gen_witness(params, ct, e, pt, sk).unwrap();
}
