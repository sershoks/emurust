//! Sortie audio : envoie les échantillons de l'émulateur à la carte son.
//!
//! L'émulateur remplit une file d'attente partagée, et le thread audio
//! la vide au rythme de la carte son. Le launcher se cale sur la taille
//! de cette file pour tourner exactement à la bonne vitesse.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample, Stream, StreamConfig};

pub struct AudioOutput {
    queue: Arc<Mutex<VecDeque<f32>>>,
    pub sample_rate: u32,
    /// Le flux doit rester vivant pour que le son continue.
    _stream: Stream,
}

impl AudioOutput {
    /// Ouvre la sortie audio par défaut. Renvoie `None` s'il n'y en a pas.
    pub fn open() -> Option<Self> {
        let device = cpal::default_host().default_output_device()?;
        let supported = device.default_output_config().ok()?;
        let format = supported.sample_format();
        let config: StreamConfig = supported.into();
        let queue = Arc::new(Mutex::new(VecDeque::new()));

        let stream = match format {
            SampleFormat::F32 => build::<f32>(&device, &config, queue.clone()),
            SampleFormat::I16 => build::<i16>(&device, &config, queue.clone()),
            SampleFormat::U16 => build::<u16>(&device, &config, queue.clone()),
            _ => None,
        }?;
        stream.play().ok()?;

        Some(AudioOutput {
            queue,
            sample_rate: config.sample_rate,
            _stream: stream,
        })
    }

    /// Nombre d'échantillons stéréo encore en attente de lecture.
    pub fn queued_frames(&self) -> usize {
        self.queue.lock().map_or(0, |q| q.len() / 2)
    }

    /// Ajoute des échantillons stéréo entrelacés à la file.
    pub fn push(&self, samples: &[f32]) {
        if let Ok(mut q) = self.queue.lock() {
            // Si on a trop d'avance (fenêtre déplacée, pause...), on repart à zéro.
            if q.len() > self.sample_rate as usize {
                q.clear();
            }
            q.extend(samples);
        }
    }
}

fn build<T>(
    device: &cpal::Device,
    config: &StreamConfig,
    queue: Arc<Mutex<VecDeque<f32>>>,
) -> Option<Stream>
where
    T: SizedSample + FromSample<f32>,
{
    let channels = config.channels as usize;
    device
        .build_output_stream::<T, _, _>(
            *config,
            move |data: &mut [T], _| {
                let mut q = queue.lock().ok();
                for frame in data.chunks_mut(channels) {
                    let (left, right) = match q.as_mut() {
                        Some(q) if q.len() >= 2 => (q.pop_front().unwrap(), q.pop_front().unwrap()),
                        _ => (0.0, 0.0),
                    };
                    let mono = (left + right) / 2.0;
                    for (i, out) in frame.iter_mut().enumerate() {
                        let v = match (channels, i) {
                            (1, _) => mono,
                            (_, 0) => left,
                            (_, 1) => right,
                            _ => mono,
                        };
                        *out = T::from_sample(v);
                    }
                }
            },
            |err| eprintln!("Erreur audio : {err}"),
            None,
        )
        .ok()
}
