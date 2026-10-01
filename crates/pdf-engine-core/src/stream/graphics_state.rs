//! Graphics State and 2D Affine Transformation Matrix according to ISO 32000-1 §8.4.
//!
//! Provides mathematically exact 2D projective transformation matrices,
//! graphics state stack management (`q` / `Q`), and text coordinate projection ($CTM \times T_m$).

/// 2D Affine Transformation Matrix in homogeneous coordinates (ISO 32000-1 §8.3.3).
///
/// Represented as a 6-element vector `[a, b, c, d, e, f]` corresponding to:
/// ```text
/// [ a  b  0 ]
/// [ c  d  0 ]
/// [ e  f  1 ]
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Matrix {
    pub a: f64,
    pub b: f64,
    pub c: f64,
    pub d: f64,
    pub e: f64,
    pub f: f64,
}

impl Default for Matrix {
    fn default() -> Self {
        Self::identity()
    }
}

impl Matrix {
    /// Constructs an identity transformation matrix.
    pub const fn identity() -> Self {
        Self {
            a: 1.0,
            b: 0.0,
            c: 0.0,
            d: 1.0,
            e: 0.0,
            f: 0.0,
        }
    }

    /// Constructs a matrix from raw 6 coefficients `[a, b, c, d, e, f]`.
    pub const fn new(a: f64, b: f64, c: f64, d: f64, e: f64, f: f64) -> Self {
        Self { a, b, c, d, e, f }
    }

    /// Constructs a translation matrix by offsets `(tx, ty)`.
    pub const fn translation(tx: f64, ty: f64) -> Self {
        Self {
            a: 1.0,
            b: 0.0,
            c: 0.0,
            d: 1.0,
            e: tx,
            f: ty,
        }
    }

    /// Constructs a scaling matrix by factors `(sx, sy)`.
    pub const fn scaling(sx: f64, sy: f64) -> Self {
        Self {
            a: 1.0 * sx,
            b: 0.0,
            c: 0.0,
            d: 1.0 * sy,
            e: 0.0,
            f: 0.0,
        }
    }

    /// Constructs a rotation matrix by an angle in radians counterclockwise.
    pub fn rotation(theta: f64) -> Self {
        let (sin_t, cos_t) = theta.sin_cos();
        Self {
            a: cos_t,
            b: sin_t,
            c: -sin_t,
            d: cos_t,
            e: 0.0,
            f: 0.0,
        }
    }

    /// Multiplies `self` by another matrix `rhs` (representing `self × rhs` in PDF postfix notation).
    ///
    /// ```text
    /// [ a1  b1  0 ]   [ a2  b2  0 ]
    /// [ c1  d1  0 ] × [ c2  d2  0 ]
    /// [ e1  f1  1 ]   [ e2  f2  1 ]
    /// ```
    pub fn multiply(&self, rhs: &Self) -> Self {
        Self {
            a: self.a * rhs.a + self.b * rhs.c,
            b: self.a * rhs.b + self.b * rhs.d,
            c: self.c * rhs.a + self.d * rhs.c,
            d: self.c * rhs.b + self.d * rhs.d,
            e: self.e * rhs.a + self.f * rhs.c + rhs.e,
            f: self.e * rhs.b + self.f * rhs.d + rhs.f,
        }
    }

    /// Transforms a 2D coordinate point `(x, y)` through this matrix.
    pub fn transform_point(&self, x: f64, y: f64) -> (f64, f64) {
        let xp = x * self.a + y * self.c + self.e;
        let yp = x * self.b + y * self.d + self.f;
        (xp, yp)
    }

    /// Computes the determinant of the matrix.
    pub fn determinant(&self) -> f64 {
        self.a * self.d - self.b * self.c
    }

    /// Inverts the affine matrix if invertible.
    pub fn inverse(&self) -> Option<Self> {
        let det = self.determinant();
        if det.abs() < 1e-12 {
            return None;
        }
        let inv_det = 1.0 / det;
        Some(Self {
            a: self.d * inv_det,
            b: -self.b * inv_det,
            c: -self.c * inv_det,
            d: self.a * inv_det,
            e: (self.c * self.f - self.d * self.e) * inv_det,
            f: (self.b * self.e - self.a * self.f) * inv_det,
        })
    }
}

/// Text State parameters active during text object processing (ISO 32000-1 §9.3).
#[derive(Debug, Clone, PartialEq)]
pub struct TextState {
    /// Active font identifier resource name (e.g. `/F1`).
    pub font_name: String,
    /// Active font size in points ($T_{fs}$).
    pub font_size: f64,
    /// Character spacing ($T_c$) added between characters in points.
    pub char_spacing: f64,
    /// Word spacing ($T_w$) added to ASCII space characters in points.
    pub word_spacing: f64,
    /// Horizontal scaling percentage ($T_h$), default 100.0.
    pub horizontal_scaling: f64,
    /// Text leading ($T_l$) distance between lines in points.
    pub leading: f64,
    /// Text rendering mode ($T_{mode}$), default 0 (fill).
    pub rendering_mode: i64,
    /// Text rise ($T_s$) for superscript/subscript adjustments in points.
    pub text_rise: f64,
}

impl Default for TextState {
    fn default() -> Self {
        Self {
            font_name: String::new(),
            font_size: 0.0,
            char_spacing: 0.0,
            word_spacing: 0.0,
            horizontal_scaling: 100.0,
            leading: 0.0,
            rendering_mode: 0,
            text_rise: 0.0,
        }
    }
}

/// Graphics State tracking device transformations, colors, and text parameters.
/// ISO 32000-1 §8.4.
#[derive(Debug, Clone, PartialEq)]
pub struct GraphicsState {
    /// Current Transformation Matrix ($CTM$).
    pub ctm: Matrix,
    /// Text Matrix ($T_m$) tracking position within a text object `BT ... ET`.
    pub text_matrix: Matrix,
    /// Text Line Matrix ($T_{lm}$) tracking start of the current text line.
    pub text_line_matrix: Matrix,
    /// Active text parameters.
    pub text_state: TextState,
}

impl Default for GraphicsState {
    fn default() -> Self {
        Self {
            ctm: Matrix::identity(),
            text_matrix: Matrix::identity(),
            text_line_matrix: Matrix::identity(),
            text_state: TextState::default(),
        }
    }
}

impl GraphicsState {
    /// Resets text matrices when encountering `BT` (Begin Text) operator (ISO 32000-1 §9.4.1).
    pub fn begin_text(&mut self) {
        self.text_matrix = Matrix::identity();
        self.text_line_matrix = Matrix::identity();
    }

    /// Concatenates a transformation matrix into the CTM (`cm` operator).
    pub fn concat_matrix(&mut self, m: &Matrix) {
        self.ctm = m.multiply(&self.ctm);
    }

    /// Moves text position by `(tx, ty)` (`Td` operator).
    pub fn move_text_position(&mut self, tx: f64, ty: f64) {
        let t = Matrix::translation(tx, ty);
        self.text_line_matrix = t.multiply(&self.text_line_matrix);
        self.text_matrix = self.text_line_matrix;
    }

    /// Sets explicit text matrix coefficients (`Tm` operator).
    pub fn set_text_matrix(&mut self, m: Matrix) {
        self.text_line_matrix = m;
        self.text_matrix = m;
    }

    /// Computes the Text Rendering Matrix ($T_{rm}$) mapping glyph coordinates to page device space.
    ///
    /// $$T_{rm} = \begin{bmatrix} T_{fs} \times \frac{T_h}{100} & 0 & 0 \\ 0 & T_{fs} & 0 \\ 0 & T_s & 1 \end{bmatrix} \times T_m \times CTM$$
    pub fn text_rendering_matrix(&self) -> Matrix {
        let scale = Matrix {
            a: self.text_state.font_size * (self.text_state.horizontal_scaling / 100.0),
            b: 0.0,
            c: 0.0,
            d: self.text_state.font_size,
            e: 0.0,
            f: self.text_state.text_rise,
        };
        scale.multiply(&self.text_matrix).multiply(&self.ctm)
    }

    /// Advances text matrix by a horizontal displacement $\Delta x$ in glyph space.
    pub fn advance_text(&mut self, dx: f64) {
        let advance = Matrix::translation(dx, 0.0);
        self.text_matrix = advance.multiply(&self.text_matrix);
    }
}

/// Graphics state stack manager supporting save (`q`) and restore (`Q`) operations.
#[derive(Debug, Clone, Default)]
pub struct GraphicsStateStack {
    /// Active state currently in effect.
    pub current: GraphicsState,
    /// Stack of saved states.
    stack: Vec<GraphicsState>,
}

impl GraphicsStateStack {
    /// Creates a new graphics state stack with default initial values.
    pub fn new() -> Self {
        Self::default()
    }

    /// Pushes a copy of the current state onto the stack (`q` operator).
    pub fn push(&mut self) {
        self.stack.push(self.current.clone());
    }

    /// Restores the most recently saved state from the stack (`Q` operator).
    pub fn pop(&mut self) -> bool {
        if let Some(saved) = self.stack.pop() {
            self.current = saved;
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_matrix_multiplication_and_transform() {
        let t = Matrix::translation(100.0, 200.0);
        let (x, y) = t.transform_point(10.0, 20.0);
        assert_eq!((x, y), (110.0, 220.0));

        let s = Matrix::scaling(2.0, 3.0);
        let (sx, sy) = s.transform_point(10.0, 20.0);
        assert_eq!((sx, sy), (20.0, 60.0));
    }

    #[test]
    fn test_graphics_state_push_pop() {
        let mut stack = GraphicsStateStack::new();
        stack.current.ctm = Matrix::translation(50.0, 50.0);
        stack.push();

        stack.current.ctm = Matrix::translation(100.0, 100.0);
        assert_eq!(stack.current.ctm.e, 100.0);

        assert!(stack.pop());
        assert_eq!(stack.current.ctm.e, 50.0);
    }
}
