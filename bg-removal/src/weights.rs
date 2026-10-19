//! Weight file loading (safetensors, f16 or f32 storage).

use crate::conv::Conv2d;
use crate::tensor::{LayerNorm, Linear};
use safetensors::{Dtype, SafeTensors};
use std::collections::HashMap;

pub struct Weights {
    map: HashMap<String, (Vec<usize>, Vec<f32>)>,
}

#[derive(Debug)]
pub struct WeightError(pub String);

impl std::fmt::Display for WeightError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for WeightError {}

impl Weights {
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, WeightError> {
        let st = SafeTensors::deserialize(bytes).map_err(|e| WeightError(format!("bad weight file: {e}")))?;
        let mut map = HashMap::new();
        for (name, view) in st.tensors() {
            let raw = view.data();
            let data: Vec<f32> = match view.dtype() {
                Dtype::F16 => raw
                    .chunks_exact(2)
                    .map(|b| half::f16::from_le_bytes([b[0], b[1]]).to_f32())
                    .collect(),
                Dtype::BF16 => raw
                    .chunks_exact(2)
                    .map(|b| half::bf16::from_le_bytes([b[0], b[1]]).to_f32())
                    .collect(),
                Dtype::F32 => raw.chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect(),
                d => return Err(WeightError(format!("{name}: unsupported dtype {d:?}"))),
            };
            map.insert(name, (view.shape().to_vec(), data));
        }
        Ok(Weights { map })
    }

    pub fn take(&mut self, name: &str) -> Result<(Vec<usize>, Vec<f32>), WeightError> {
        self.map.remove(name).ok_or_else(|| WeightError(format!("missing weight {name}")))
    }

    pub fn vec(&mut self, name: &str) -> Result<Vec<f32>, WeightError> {
        Ok(self.take(name)?.1)
    }

    pub fn conv(&mut self, prefix: &str, stride: usize, pad: usize, dil: usize, bias: bool) -> Result<Conv2d, WeightError> {
        let (shape, w) = self.take(&format!("{prefix}.weight"))?;
        if shape.len() != 4 || shape[2] != shape[3] {
            return Err(WeightError(format!("{prefix}: unexpected conv shape {shape:?}")));
        }
        let b = if bias { Some(self.vec(&format!("{prefix}.bias"))?) } else { None };
        Ok(Conv2d::new(w, b, shape[0], shape[1], shape[2], stride, pad, dil))
    }

    pub fn linear(&mut self, prefix: &str, bias: bool) -> Result<Linear, WeightError> {
        let (shape, w_t) = self.take(&format!("{prefix}.weight_t"))?;
        let b = if bias { Some(self.vec(&format!("{prefix}.bias"))?) } else { None };
        Ok(Linear { w_t, b, cin: shape[0], cout: shape[1] })
    }

    pub fn layer_norm(&mut self, prefix: &str) -> Result<LayerNorm, WeightError> {
        Ok(LayerNorm { w: self.vec(&format!("{prefix}.weight"))?, b: self.vec(&format!("{prefix}.bias"))? })
    }

    pub fn remaining(&self) -> Vec<&String> {
        let mut v: Vec<_> = self.map.keys().collect();
        v.sort();
        v
    }
}
