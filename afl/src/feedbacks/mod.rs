use alloc::string::String;
use alloc::string::ToString;
use alloc::vec::Vec;
use core::marker::PhantomData;
use num::Integer;

use crate::inputs::Input;
use crate::observers::observer_serde::NamedSerdeAnyMap;
use crate::observers::MapObserver;
use crate::AflError;
use crate::{corpus::Testcase, observers::Observer};

pub type MaxMapFeedback<T, O> = MapFeedback<T, MaxReducer<T>, O>;
pub type MinMapFeedback<T, O> = MapFeedback<T, MinReducer<T>, O>;

//pub type MaxMapTrackerFeedback<T, O> = MapFeedback<T, MaxReducer<T>, O>;
//pub type MinMapTrackerFeedback<T, O> = MapFeedback<T, MinReducer<T>, O>;

/// Feedbacks evaluate the observers.
/// Basically, they reduce the information provided by an observer to a value,
/// indicating the "interestingness" of the last run.
pub trait Feedback<I>
where
    I: Input,
{
    /// is_interesting should return the "Interestingness" from 0 to 255 (percent times 2.55)
    fn is_interesting(&mut self, input: &I, observers: &NamedSerdeAnyMap) -> Result<u32, AflError>;

    /// Append to the testcase the generated metadata in case of a new corpus item
    #[inline]
    fn append_metadata(&mut self, _testcase: &mut Testcase<I>) -> Result<(), AflError> {
        Ok(())
    }

    /// Discard the stored metadata in case that the testcase is not added to the corpus
    #[inline]
    fn discard_metadata(&mut self, _input: &I) -> Result<(), AflError> {
        Ok(())
    }

    /// The name of this feedback
    fn name(&self) -> &String;
}

/// A Reducer function is used to aggregate values for the novelty search
pub trait Reducer<T>
where
    T: Integer + Copy + 'static,
{
    fn reduce(first: T, second: T) -> T;
}

pub struct MaxReducer<T>
where
    T: Integer + Copy + 'static,
{
    phantom: PhantomData<T>,
}

impl<T> Reducer<T> for MaxReducer<T>
where
    T: Integer + Copy + 'static,
{
    #[inline]
    fn reduce(first: T, second: T) -> T {
        if first > second {
            first
        } else {
            second
        }
    }
}

pub struct MinReducer<T>
where
    T: Integer + Copy + 'static,
{
    phantom: PhantomData<T>,
}

impl<T> Reducer<T> for MinReducer<T>
where
    T: Integer + Copy + 'static,
{
    #[inline]
    fn reduce(first: T, second: T) -> T {
        if first < second {
            first
        } else {
            second
        }
    }
}

/// The most common AFL-like feedback type
pub struct MapFeedback<T, R, O>
where
    T: Integer + Default + Copy + 'static,
    R: Reducer<T>,
    O: MapObserver<T>,
{
    /// Contains information about untouched entries
    history_map: Vec<T>,
    /// Name identifier of this instance
    name: String,
    /// Phantom Data of Reducer
    phantom: PhantomData<(R, O)>,
}

impl<T, R, O, I> Feedback<I> for MapFeedback<T, R, O>
where
    T: Integer + Default + Copy + 'static,
    R: Reducer<T>,
    O: MapObserver<T> + 'static,
    I: Input,
{
    fn is_interesting(
        &mut self,
        _input: &I,
        observers: &NamedSerdeAnyMap,
    ) -> Result<u32, AflError> {
        let mut interesting = 0;
        // TODO optimize
        let observer = observers.get::<O>(&self.name).unwrap();
        let size = observer.map().len();
        for i in 0..size {
            let history = self.history_map[i];
            let item = observer.map()[i];
            let reduced = R::reduce(history, item);
            if history != reduced {
                self.history_map[i] = reduced;
                interesting += 1;
            }
        }

        Ok(interesting)
    }

    #[inline]
    fn name(&self) -> &String {
        &self.name
    }
}

impl<T, R, O> MapFeedback<T, R, O>
where
    T: Integer + Default + Copy + 'static,
    R: Reducer<T>,
    O: MapObserver<T> + Observer,
{
    /// Create new MapFeedback
    pub fn new(name: &'static str, map_size: usize) -> Self {
        Self {
            history_map: vec![T::default(); map_size],
            name: name.to_string(),
            phantom: PhantomData,
        }
    }

    pub fn new_with_observer(map_observer: &O) -> Self {
        Self {
            history_map: vec![T::default(); map_observer.map().len()],
            name: map_observer.name().to_string(),
            phantom: PhantomData,
        }
    }
}

impl<T, R, O> MapFeedback<T, R, O>
where
    T: Integer + Default + Copy + 'static,
    R: Reducer<T>,
    O: MapObserver<T>,
{
    /// Create new MapFeedback using a map observer, and a map.
    /// The map can be shared.
    pub fn with_history_map(name: &'static str, history_map: Vec<T>) -> Self {
        Self {
            history_map: history_map,
            name: name.into(),
            phantom: PhantomData,
        }
    }
}

/*
#[derive(Serialize, Deserialize)]
pub struct MapNoveltiesMetadata {
    novelties: Vec<usize>,
}

impl SerdeAny for MapNoveltiesMetadata {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

impl TestcaseMetadata for MapNoveltiesMetadata {
    fn name(&self) -> &'static str {
        "MapNoveltiesMetadata"
    }
}
impl MapNoveltiesMetadata {
    pub fn novelties(&self) -> &[usize] {
        &self.novelties
    }

    pub fn new(novelties: Vec<usize>) -> Self {
        Self {
            novelties: novelties,
        }
    }
}

/// The most common AFL-like feedback type that adds metadata about newly discovered entries
pub struct MapTrackerFeedback<T, R, O>
where
    T: Integer + Copy + 'static,
    R: Reducer<T>,
    O: MapObserver<T>,
{
    /// Contains information about untouched entries
    history_map: Vec<T>,
    /// Name identifier of this instance
    name: &'static str,
    /// Phantom Data of Reducer
    phantom: PhantomData<(R, O)>,
    /// Track novel entries indexes
    novelties: Vec<usize>,
}

impl<T, R, O, I> Feedback<I> for MapTrackerFeedback<T, R, O>
where
    T: Integer + Copy + 'static,
    R: Reducer<T>,
    O: MapObserver<T>,
    I: Input,
{
    fn is_interesting(&mut self, _input: &I) -> Result<u32, AflError> {
        let mut interesting = 0;

        // TODO optimize
        let size = self.map_observer.borrow().map().len();
        let mut history_map = self.history_map.borrow_mut();
        let observer = self.map_observer.borrow();
        for i in 0..size {
            let history = history_map[i];
            let item = observer.map()[i];
            let reduced = R::reduce(history, item);
            if history != reduced {
                history_map[i] = reduced;
                interesting += 1;
                self.novelties.push(i);
            }
        }

        Ok(interesting)
    }

    fn append_metadata(&mut self, testcase: &mut Testcase<I>) -> Result<(), AflError> {
        let meta = MapNoveltiesMetadata::new(core::mem::take(&mut self.novelties));
        testcase.add_metadata(meta);
        Ok(())
    }

    /// Discard the stored metadata in case that the testcase is not added to the corpus
    fn discard_metadata(&mut self, _input: &I) -> Result<(), AflError> {
        self.novelties.clear();
        Ok(())
    }

    fn name(&self) -> &'static str {
        self.name
    }
}

impl<T, R, O> MapTrackerFeedback<T, R, O>
where
    T: Integer + Copy + Default + 'static,
    R: Reducer<T>,
    O: MapObserver<T>,
{
    /// Create new MapFeedback using a map observer
    pub fn new(map_observer: Rc<RefCell<O>>, map_size: usize) -> Self {
        Self {
            map_observer: map_observer,
            history_map: create_history_map::<T>(map_size),
            phantom: PhantomData,
            novelties: vec![],
        }
    }
}

impl<T, R, O> MapTrackerFeedback<T, R, O>
where
    T: Integer + Copy + 'static,
    R: Reducer<T>,
    O: MapObserver<T>,
{
    /// Create new MapFeedback using a map observer, and a map.
    /// The map can be shared.
    pub fn with_history_map(
        map_observer: Rc<RefCell<O>>,
        history_map: Rc<RefCell<Vec<T>>>,
    ) -> Self {
        MapTrackerFeedback {
            map_observer: map_observer,
            history_map: history_map,
            phantom: PhantomData,
            novelties: vec![],
        }
    }
}
*/
