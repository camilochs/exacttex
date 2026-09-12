# Predictions, registered before the second campaign runs

**Registered 2026-09-11**, before a single paper beyond the inherited fifty was
downloaded, and before any experiment ran against them. The numbers on the right
of each prediction are the first campaign's (fifty papers, 2026-09-01,
`../corpus/RESULTS.md`); the prediction is what the same measurement should say
on two hundred.

A prediction that turns out wrong is the point: it is the only way this campaign
can teach anything the first one did not.

## The harvest itself

1. **The licence gate decides the shape of the corpus.** In computer science it
   accepted about half the candidates (50 accepted against 51 refused for the
   licence). Outside computer science authors choose Creative Commons far less
   often, so I predict an acceptance rate **below 30%** in maths and physics, and
   a corpus that ends up **computer-science heavy even after stratifying**: at
   least 45% of the two hundred from the `cs` field.
2. **Templates are the axis that pays.** The inherited fifty are `article` (22),
   `acmart` (15), `IEEEtran` (4), `elsarticle` (3), one `book`, one `eptcs`. I
   predict the campaign brings at least **six document classes never seen before**,
   among them at least one of `revtex4-2`, `amsart` or an `aastex` variant.

## E1 — specificity: an unmodified `.tex` renamed to `.xtex`

3. **Byte identity holds.** On the first corpus: 381 of 381 files exit 0 with no
   diagnostics and emit byte-identical output. I predict **at least 99% of files**
   do the same here, and that **every failure, if any, comes from a document class
   absent from the first fifty**. A failure in `article` or `acmart` would mean the
   first campaign got lucky, and would be the most interesting result of the two.

## E2 — the mechanical annotation ramp

4. **Real defects in published papers stay near one paper in five.** First corpus:
   9 of 50 papers (18%) carried a real defect — duplicate labels, missing citation
   keys, broken references. I predict **between 12% and 26%** of the two hundred.
5. **False errors per paper do not grow.** On the fixed commit the first corpus
   produced 143 false errors over 50 papers, concentrated in two rules. I predict
   **no more than 3 false errors per paper on average**, and that **the two known
   rules (`\\[` read as display math, `sec:` after `\appendix`) account for more
   than half** of them.

## E5 — prevalence of the prefix/class disagreement

6. **The rate holds and the interval halves.** First corpus: 14 of 1,121
   declarations, 12.5 per 1,000, Wilson interval 7.5 to 20.9. I predict a rate
   **between 6 and 20 per 1,000** with an interval **narrower than 8 points wide**.
7. **The clustering does not repeat.** Thirteen of the fourteen true positives came
   from one thesis-length paper. I predict that on two hundred papers **no single
   paper contributes more than half** of the true positives. If one does again, the
   honest reading is that this defect is a property of long documents, not of
   documents, and the paper must say so.

## E3 — planted defects, against the incumbents (subsample of 80)

8. **Recall is saturated and will not move.** I predict every class within **ten
   points** of the first corpus, and the wrong-class defect (A4) still **caught by
   ExactTeX and missed by texlab and TeX on every twin**.

## E6 — external verification (subsample of 40)

9. **The registries answer about a third.** First corpus, ten bibliographies:
   31% verified, 28% partial, 2% mismatch, 39% unverified. I predict **verified
   between 25% and 40%** and **unverified between 30% and 45%**, and that the
   unverified share is **higher outside computer science**, where older works and
   books are cited more often.

## The compiler itself

10. **More templates, more defects.** The first campaign produced ten defect
    reproducers from fifty papers. Four times the papers with wider templates should
    not give four times the defects, because the common shapes are already covered.
    I predict **between 8 and 20 new reproducers**, and that **most come from
    document classes or packages the first corpus never met**.

## What would make me stop and rethink

- Byte identity failing on `article` or `acmart` (prediction 3): the gradual
  guarantee is the paper's foundation, and a failure there is not a bug report, it
  is a different paper.
- A false-error rate above 3 per paper (prediction 5): the check would be noisier
  than the defect it reports, and the prefix rule would have to be rewritten before
  anything is published.
