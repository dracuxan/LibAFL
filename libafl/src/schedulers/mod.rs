//! Schedule the access to the Corpus.

use alloc::{borrow::ToOwned, string::ToString};
use core::marker::PhantomData;

pub mod testcase_score;
pub use testcase_score::{LenTimeMulTestcaseScore, TestcaseScore};

pub mod queue;
pub use queue::QueueScheduler;

pub mod minimizer;
pub use minimizer::{
    IndexesLenTimeMinimizerScheduler, LenTimeMinimizerScheduler, MinimizerScheduler,
};

pub mod powersched;
pub use powersched::{PowerQueueScheduler, SchedulerMetadata};

pub mod probabilistic_sampling;
pub use probabilistic_sampling::ProbabilitySamplingScheduler;

pub mod accounting;
pub use accounting::CoverageAccountingScheduler;

pub mod weighted;
pub use weighted::{StdWeightedScheduler, WeightedScheduler};

pub mod tuneable;
use libafl_bolts::{
    rands::Rand,
    tuples::{Handle, MatchNameRef},
};
pub use tuneable::*;

use crate::{
    corpus::{Corpus, CorpusId, HasTestcase, SchedulerTestcaseMetadata, Testcase},
    inputs::Input,
    observers::{MapObserver, ObserversTuple},
    random_corpus_id,
    state::{HasCorpus, HasRand, State},
    Error, HasMetadata,
};

/// The scheduler also implements `on_remove` and `on_replace` if it implements this stage.
pub trait RemovableScheduler<I, S>
where
    I: Input,
{
    /// Removed the given entry from the corpus at the given index
    /// When you remove testcases, make sure that that testcase is not currently fuzzed one!
    fn on_remove(
        &mut self,
        _state: &mut S,
        _id: CorpusId,
        _testcase: &Option<Testcase<I>>,
    ) -> Result<(), Error> {
        Ok(())
    }

    /// Replaced the given testcase at the given idx
    fn on_replace(
        &mut self,
        _state: &mut S,
        _id: CorpusId,
        _prev: &Testcase<I>,
    ) -> Result<(), Error> {
        Ok(())
    }
}

/// Defines the common metadata operations for the AFL-style schedulers
pub trait AflScheduler<I, O, S>
where
    S: HasCorpus + HasMetadata + HasTestcase,
    O: MapObserver,
{
    /// The type of [`MapObserver`] that this scheduler will use as reference
    type MapObserverRef: AsRef<O>;

    /// Return the last hash
    fn last_hash(&self) -> usize;

    /// Set the last hash
    fn set_last_hash(&mut self, value: usize);

    /// Get the observer map observer name
    fn map_observer_handle(&self) -> &Handle<Self::MapObserverRef>;

    /// Called when a [`Testcase`] is added to the corpus
    fn on_add_metadata(&self, state: &mut S, id: CorpusId) -> Result<(), Error> {
        let current_id = *state.corpus().current();

        let mut depth = match current_id {
            Some(parent_idx) => state
                .testcase(parent_idx)?
                .metadata::<SchedulerTestcaseMetadata>()?
                .depth(),
            None => 0,
        };

        // TODO increase perf_score when finding new things like in AFL
        // https://github.com/google/AFL/blob/master/afl-fuzz.c#L6547

        // Attach a `SchedulerTestcaseMetadata` to the queue entry.
        depth += 1;
        let mut testcase = state.testcase_mut(id)?;
        testcase.add_metadata(SchedulerTestcaseMetadata::with_n_fuzz_entry(
            depth,
            self.last_hash(),
        ));
        testcase.set_parent_id_optional(current_id);
        Ok(())
    }

    /// Called when a [`Testcase`] is evaluated
    fn on_evaluation_metadata<OT>(
        &mut self,
        state: &mut S,
        _input: &I,
        observers: &OT,
    ) -> Result<(), Error>
    where
        OT: ObserversTuple<S>,
    {
        let observer = observers
            .get(self.map_observer_handle())
            .ok_or_else(|| Error::key_not_found("MapObserver not found".to_string()))?
            .as_ref();

        let mut hash = observer.hash_simple() as usize;

        let psmeta = state.metadata_mut::<SchedulerMetadata>()?;

        hash %= psmeta.n_fuzz().len();
        // Update the path frequency
        psmeta.n_fuzz_mut()[hash] = psmeta.n_fuzz()[hash].saturating_add(1);

        self.set_last_hash(hash);

        Ok(())
    }

    /// Called when choosing the next [`Testcase`]
    fn on_next_metadata(&mut self, state: &mut S, _next_id: Option<CorpusId>) -> Result<(), Error> {
        let current_id = *state.corpus().current();

        if let Some(id) = current_id {
            let mut testcase = state.testcase_mut(id)?;
            let tcmeta = testcase.metadata_mut::<SchedulerTestcaseMetadata>()?;

            if tcmeta.handicap() >= 4 {
                tcmeta.set_handicap(tcmeta.handicap() - 4);
            } else if tcmeta.handicap() > 0 {
                tcmeta.set_handicap(tcmeta.handicap() - 1);
            }
        }

        Ok(())
    }
}

/// Trait for Schedulers which track queue cycles
pub trait HasQueueCycles {
    /// The amount of cycles the scheduler has completed.
    fn queue_cycles(&self) -> u64;
}

/// The scheduler define how the fuzzer requests a testcase from the corpus.
/// It has hooks to corpus add/replace/remove to allow complex scheduling algorithms to collect data.
pub trait Scheduler<I, S>
where
    S: HasCorpus,
{
    /// Called when a [`Testcase`] is added to the corpus
    fn on_add(&mut self, _state: &mut S, _id: CorpusId) -> Result<(), Error>;
    // Add parent_id here if it has no inner

    /// An input has been evaluated
    fn on_evaluation<OT>(
        &mut self,
        _state: &mut S,
        _input: &I,
        _observers: &OT,
    ) -> Result<(), Error>
    where
        OT: ObserversTuple<S>,
    {
        Ok(())
    }

    /// Gets the next entry
    fn next(&mut self, state: &mut S) -> Result<CorpusId, Error>;
    // Increment corpus.current() here if it has no inner

    /// Set current fuzzed corpus id and `scheduled_count`
    fn set_current_scheduled(
        &mut self,
        state: &mut S,
        next_id: Option<CorpusId>,
    ) -> Result<(), Error> {
        *state.corpus_mut().current_mut() = next_id;
        Ok(())
    }
}

/// Feed the fuzzer simply with a random testcase on request
#[derive(Debug, Clone)]
pub struct RandScheduler<S> {
    phantom: PhantomData<S>,
}

impl<I, S> Scheduler<I, S> for RandScheduler<S>
where
    S: HasCorpus + HasRand + HasTestcase + State,
{
    fn on_add(&mut self, state: &mut S, id: CorpusId) -> Result<(), Error> {
        // Set parent id
        let current_id = *state.corpus().current();
        state
            .corpus()
            .get(id)?
            .borrow_mut()
            .set_parent_id_optional(current_id);

        Ok(())
    }

    /// Gets the next entry at random
    fn next(&mut self, state: &mut S) -> Result<CorpusId, Error> {
        if state.corpus().count() == 0 {
            Err(Error::empty(
                "No entries in corpus. This often implies the target is not properly instrumented."
                    .to_owned(),
            ))
        } else {
            let id = random_corpus_id!(state.corpus(), state.rand_mut());
            <Self as Scheduler<I, S>>::set_current_scheduled(self, state, Some(id))?;
            Ok(id)
        }
    }
}

impl<S> RandScheduler<S> {
    /// Create a new [`RandScheduler`] that just schedules randomly.
    #[must_use]
    pub fn new() -> Self {
        Self {
            phantom: PhantomData,
        }
    }
}

impl<S> Default for RandScheduler<S> {
    fn default() -> Self {
        Self::new()
    }
}

/// A [`StdScheduler`] uses the default scheduler in `LibAFL` to schedule [`Testcase`]s.
///
/// The current `Std` is a [`RandScheduler`], although this may change in the future, if another [`Scheduler`] delivers better results.
pub type StdScheduler<S> = RandScheduler<S>;
