//! Simple fixed-size bit set.

#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct BitSet {
    words: Vec<u64>,
}

impl BitSet {
    pub fn new(n: usize) -> BitSet {
        BitSet { words: vec![0; (n + 63) / 64] }
    }
    pub fn full(n: usize) -> BitSet {
        let mut b = BitSet { words: vec![!0; (n + 63) / 64] };
        if n % 64 != 0 {
            if let Some(l) = b.words.last_mut() {
                *l = (1u64 << (n % 64)) - 1;
            }
        }
        b
    }
    #[inline]
    pub fn insert(&mut self, i: usize) -> bool {
        let (w, b) = (i / 64, 1u64 << (i % 64));
        let was = self.words[w] & b != 0;
        self.words[w] |= b;
        !was
    }
    #[inline]
    pub fn remove(&mut self, i: usize) {
        self.words[i / 64] &= !(1u64 << (i % 64));
    }
    #[inline]
    pub fn contains(&self, i: usize) -> bool {
        self.words.get(i / 64).map_or(false, |w| w & (1u64 << (i % 64)) != 0)
    }
    pub fn union_with(&mut self, o: &BitSet) -> bool {
        let mut changed = false;
        for (a, b) in self.words.iter_mut().zip(&o.words) {
            let n = *a | *b;
            if n != *a {
                *a = n;
                changed = true;
            }
        }
        changed
    }
    pub fn intersect_with(&mut self, o: &BitSet) -> bool {
        let mut changed = false;
        for (a, b) in self.words.iter_mut().zip(&o.words) {
            let n = *a & *b;
            if n != *a {
                *a = n;
                changed = true;
            }
        }
        changed
    }
    pub fn subtract(&mut self, o: &BitSet) {
        for (a, b) in self.words.iter_mut().zip(&o.words) {
            *a &= !*b;
        }
    }
    pub fn is_empty(&self) -> bool {
        self.words.iter().all(|w| *w == 0)
    }
    pub fn clear(&mut self) {
        for w in &mut self.words {
            *w = 0;
        }
    }
    pub fn iter(&self) -> impl Iterator<Item = usize> + '_ {
        self.words.iter().enumerate().flat_map(|(wi, &w)| {
            let mut w = w;
            std::iter::from_fn(move || {
                if w == 0 {
                    return None;
                }
                let t = w.trailing_zeros() as usize;
                w &= w - 1;
                Some(wi * 64 + t)
            })
        })
    }
    pub fn count(&self) -> usize {
        self.words.iter().map(|w| w.count_ones() as usize).sum()
    }
}
