//! The kd-tree the node set builds for its neighbour lists (0x482190 /
//! 0x4822b0): the engine links the ANN library (0x5b9130 constructor,
//! 0x5b8fe0 recursive build, 0x5bb2b0 the split rule selected by code 5
//! through the table at 0x5b92f0, 0x5b97b0 `annkSearch`, 0x5b9470 /
//! 0x5b95e0 the split and leaf searches, 0x5b9310 the sorted result list).
//! The neighbour lists keep the order the search returns, and equal
//! distances are ordered by the traversal, so the tree is rebuilt exactly:
//! two dimensions, bucket size one, the sliding midpoint split, and the
//! search with eps = 1e-12.

/// A point index list node.
enum Node {
    Empty,
    Leaf(Vec<usize>),
    Split {
        cd: usize,
        cv: f64,
        lo: f64,
        hi: f64,
        left: Box<Node>,
        right: Box<Node>,
    },
}

pub struct KdTree {
    pts: Vec<[f64; 2]>,
    root: Node,
    box_lo: [f64; 2],
    box_hi: [f64; 2],
}

const DIM: usize = 2;
const BUCKET: usize = 1;
/// ANN_DIST_INF.
const INF: f64 = f64::MAX;

fn pa(pts: &[[f64; 2]], pidx: &[usize], i: usize, d: usize) -> f64 {
    pts[pidx[i]][d]
}

/// 0x5ba970: max - min of the coordinate `d` over the points.
fn spread(pts: &[[f64; 2]], pidx: &[usize], d: usize) -> f64 {
    let (min, max) = min_max(pts, pidx, d);
    max - min
}

/// 0x5baad0.
fn min_max(pts: &[[f64; 2]], pidx: &[usize], d: usize) -> (f64, f64) {
    let (mut min, mut max) = (pa(pts, pidx, 0, d), pa(pts, pidx, 0, d));
    for i in 1..pidx.len() {
        let v = pa(pts, pidx, i, d);
        if v < min {
            min = v;
        } else if v > max {
            max = v;
        }
    }
    (min, max)
}

/// 0x5ba660: the enclosing rectangle.
fn encl_rect(pts: &[[f64; 2]], pidx: &[usize]) -> ([f64; 2], [f64; 2]) {
    let mut lo = [0.0; 2];
    let mut hi = [0.0; 2];
    for d in 0..DIM {
        let (min, max) = min_max(pts, pidx, d);
        lo[d] = min;
        hi[d] = max;
    }
    (lo, hi)
}

/// 0x5baed0: partition into `< cv`, `== cv`, `> cv`; returns the two
/// boundaries.
fn plane_split(pts: &[[f64; 2]], pidx: &mut [usize], d: usize, cv: f64) -> (usize, usize) {
    let n = pidx.len() as i32;
    let mut l = 0i32;
    let mut r = n - 1;
    loop {
        while l < n && pa(pts, pidx, l as usize, d) < cv {
            l += 1;
        }
        while r >= 0 && pa(pts, pidx, r as usize, d) >= cv {
            r -= 1;
        }
        if l > r {
            break;
        }
        pidx.swap(l as usize, r as usize);
        l += 1;
        r -= 1;
    }
    let br1 = l;
    r = n - 1;
    loop {
        while l < n && pa(pts, pidx, l as usize, d) <= cv {
            l += 1;
        }
        while r >= br1 && pa(pts, pidx, r as usize, d) > cv {
            r -= 1;
        }
        if l > r {
            break;
        }
        pidx.swap(l as usize, r as usize);
        l += 1;
        r -= 1;
    }
    (br1 as usize, l as usize)
}

/// 0x5bb2b0, the split rule of code 5 (the library's suggested rule): among
/// the box sides at least 0.999 of the longest, the dimension with the
/// largest point spread is cut at the box midpoint; when every point lies on
/// one side the cut slides to the nearest point, and the plane split's
/// boundaries pick the side the equal points join.
fn midpoint_split(
    pts: &[[f64; 2]],
    pidx: &mut [usize],
    lo: &[f64; 2],
    hi: &[f64; 2],
) -> (usize, f64, usize) {
    let n = pidx.len();
    let mut max_len = hi[0] - lo[0];
    for d in 1..DIM {
        let len = hi[d] - lo[d];
        if len > max_len {
            max_len = len;
        }
    }
    let threshold = max_len * 0.999;
    let mut max_spread = -1.0f64;
    let mut cut_dim = 0usize;
    for d in 0..DIM {
        if !((hi[d] - lo[d]) < threshold) {
            let spr = spread(pts, pidx, d);
            if spr > max_spread {
                max_spread = spr;
                cut_dim = d;
            }
        }
    }
    let ideal = (lo[cut_dim] + hi[cut_dim]) * 0.5;
    let (min, max) = min_max(pts, pidx, cut_dim);
    let cv = if ideal < min {
        min
    } else if ideal <= max {
        ideal
    } else {
        max
    };
    let (br1, br2) = plane_split(pts, pidx, cut_dim, cv);
    let n_lo = if ideal < min {
        1
    } else if ideal > max {
        n - 1
    } else if br1 > n / 2 {
        br1
    } else if br2 < n / 2 {
        br2
    } else {
        n / 2
    };
    (cut_dim, cv, n_lo)
}

/// 0x5b8fe0.
fn build(pts: &[[f64; 2]], pidx: &mut [usize], lo: &mut [f64; 2], hi: &mut [f64; 2]) -> Node {
    let n = pidx.len();
    if n <= BUCKET {
        if n == 0 {
            return Node::Empty;
        }
        return Node::Leaf(pidx.to_vec());
    }
    let (cd, cv, n_lo) = midpoint_split(pts, pidx, lo, hi);
    let lv = lo[cd];
    let hv = hi[cd];
    hi[cd] = cv;
    let left = build(pts, &mut pidx[..n_lo], lo, hi);
    hi[cd] = hv;
    lo[cd] = cv;
    let right = build(pts, &mut pidx[n_lo..], lo, hi);
    lo[cd] = lv;
    Node::Split {
        cd,
        cv,
        lo: lv,
        hi: hv,
        left: Box::new(left),
        right: Box::new(right),
    }
}

/// 0x5b9310: the k best (distance, index) pairs, sorted; an equal distance
/// goes after the ones already there.
struct MinK {
    k: usize,
    items: Vec<(f64, usize)>,
}

impl MinK {
    fn max_key(&self) -> f64 {
        if self.items.len() == self.k {
            self.items[self.k - 1].0
        } else {
            INF
        }
    }
    fn insert(&mut self, key: f64, info: usize) {
        let mut i = self.items.len();
        self.items.push((key, info));
        while i > 0 && self.items[i - 1].0 > key {
            self.items[i] = self.items[i - 1];
            i -= 1;
        }
        self.items[i] = (key, info);
        if self.items.len() > self.k {
            self.items.pop();
        }
    }
}

struct Search<'a> {
    pts: &'a [[f64; 2]],
    q: [f64; 2],
    max_err: f64,
    found: MinK,
}

impl Search<'_> {
    fn update_box_dist(diff: f64, cut_diff: f64, box_dist: f64) -> f64 {
        let mut box_diff = diff;
        if box_diff < 0.0 {
            box_diff = 0.0;
        }
        ((cut_diff * cut_diff) - (box_diff * box_diff)) + box_dist
    }

    /// 0x5b9470 / 0x5b95e0.
    fn visit(&mut self, node: &Node, box_dist: f64) {
        match node {
            Node::Empty => {}
            Node::Leaf(idx) => {
                let mut min_dist = self.found.max_key();
                for &p in idx {
                    let mut dist = 0.0f64;
                    let mut d = 0;
                    while d < DIM {
                        let t = self.q[d] - self.pts[p][d];
                        dist += t * t;
                        if dist > min_dist {
                            break;
                        }
                        d += 1;
                    }
                    if d >= DIM {
                        self.found.insert(dist, p);
                        min_dist = self.found.max_key();
                    }
                }
            }
            Node::Split {
                cd,
                cv,
                lo,
                hi,
                left,
                right,
            } => {
                let cut_diff = self.q[*cd] - cv;
                if cut_diff < 0.0 {
                    self.visit(left, box_dist);
                    let box_dist = Self::update_box_dist(lo - self.q[*cd], cut_diff, box_dist);
                    if (box_dist * self.max_err) < self.found.max_key() {
                        self.visit(right, box_dist);
                    }
                } else {
                    self.visit(right, box_dist);
                    let box_dist = Self::update_box_dist(self.q[*cd] - hi, cut_diff, box_dist);
                    if (box_dist * self.max_err) < self.found.max_key() {
                        self.visit(left, box_dist);
                    }
                }
            }
        }
    }
}

impl KdTree {
    /// 0x5b9130 over the points in index order.
    pub fn new(pts: Vec<[f64; 2]>) -> KdTree {
        let n = pts.len();
        if n == 0 {
            return KdTree {
                pts,
                root: Node::Empty,
                box_lo: [0.0; 2],
                box_hi: [0.0; 2],
            };
        }
        let mut pidx: Vec<usize> = (0..n).collect();
        let (mut lo, mut hi) = encl_rect(&pts, &pidx);
        let (box_lo, box_hi) = (lo, hi);
        let root = build(&pts, &mut pidx, &mut lo, &mut hi);
        KdTree {
            pts,
            root,
            box_lo,
            box_hi,
        }
    }

    /// 0x5b97b0: the `k` nearest points to `q`, nearest first, as
    /// (squared distance, index).
    pub fn nearest(&self, q: [f64; 2], k: usize, eps: f64) -> Vec<(f64, usize)> {
        assert!(k <= self.pts.len(), "ANN: k larger than the point count");
        let scale = eps + 1.0;
        let mut search = Search {
            pts: &self.pts,
            q,
            max_err: scale * scale,
            found: MinK {
                k,
                items: Vec::with_capacity(k + 1),
            },
        };
        // 0x5ba7e0: the squared distance from the query to the bounding box.
        let mut box_dist = 0.0f64;
        for (d, &qd) in q.iter().enumerate().take(DIM) {
            if qd < self.box_lo[d] {
                let t = self.box_lo[d] - qd;
                box_dist += t * t;
            } else if qd > self.box_hi[d] {
                let t = qd - self.box_hi[d];
                box_dist += t * t;
            }
        }
        search.visit(&self.root, box_dist);
        search.found.items
    }
}

/// The tree in pre-order as the host's `--aa-kd-tree` emits it: 1 cd cv lo
/// hi for a split, 2 n indices for a leaf, 3 for the empty leaf.
#[cfg(test)]
pub(crate) fn preorder(tree: &KdTree) -> Vec<f64> {
    fn go(n: &Node, out: &mut Vec<f64>) {
        match n {
            Node::Empty => out.push(3.0),
            Node::Leaf(i) => {
                out.push(2.0);
                out.push(i.len() as f64);
                out.extend(i.iter().map(|&k| k as f64));
            }
            Node::Split {
                cd,
                cv,
                lo,
                hi,
                left,
                right,
            } => {
                out.extend([1.0, *cd as f64, *cv, *lo, *hi]);
                go(left, out);
                go(right, out);
            }
        }
    }
    let mut out = Vec::new();
    go(&tree.root, &mut out);
    out
}
