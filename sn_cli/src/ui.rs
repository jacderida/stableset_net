// Copyright 2023 MaidSafe.net limited.
//
// This SAFE Network Software is licensed to you under The General Public License (GPL), version 3.
// Unless required by applicable law or agreed to in writing, the SAFE Network Software distributed
// under the GPL Licence is distributed on an "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied. Please review the Licences for the specific language governing
// permissions and limitations relating to use of the SAFE Network Software.

use color_eyre::Result;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use sn_client::{Client, ClientEvent};
use std::time::Duration;

pub struct UiManager {
    client: Client,
    multi_bar: MultiProgress,
    safe_task_progress_bar: ProgressBar,
    safe_text_bar: ProgressBar,
    client_text_bar: ProgressBar,
}

impl UiManager {
    pub fn new(client: Client) -> Result<Self> {
        let multi_bar = MultiProgress::new();
        let safe_task_progress_bar = multi_bar.add(ProgressBar::new(0));
        safe_task_progress_bar.set_style(
            ProgressStyle::default_bar()
                .template("{spinner:.green} [{bar:40.cyan/blue}] {pos}/{len}")?
                .progress_chars("#>-"),
        );
        let safe_text_bar = multi_bar.add(ProgressBar::new(0));
        safe_text_bar.set_style(ProgressStyle::default_bar().template("{msg}")?);
        let client_text_bar = multi_bar.add(ProgressBar::new(0));
        client_text_bar.set_style(ProgressStyle::default_bar().template("Client: {msg}")?);
        Ok(Self {
            client,
            multi_bar,
            safe_task_progress_bar,
            safe_text_bar,
            client_text_bar,
        })
    }

    pub async fn run(&self) {
        self.safe_task_progress_bar
            .enable_steady_tick(Duration::from_millis(100));
        let mut events_channel = self.client.events_channel();
        loop {
            let event_receiver = events_channel.recv();
            match event_receiver.await {
                Ok(event) => match event {
                    ClientEvent::ConnectedToNetwork => {
                        self.set_client_feedback("Connected to the network");
                    }
                    ClientEvent::GossipsubMsg { topic, .. } => {
                        self.set_client_feedback(&format!("Received message on topic {}", topic));
                    }
                    ClientEvent::InactiveClient(duration) => {
                        self.set_client_feedback(&format!("Inactive client for {:?}", duration));
                    }
                    ClientEvent::ObtainStoreCostFailed(error_msg) => {
                        self.set_client_feedback(&error_msg);
                    }
                },
                Err(_) => {
                    self.set_client_feedback("Event channel closed. Exiting...");
                    break;
                }
            }
        }
    }

    /// Initialise the progress bar that tracks a task initiated and coordinated by safe.
    ///
    /// Provide the title and size of the task, e.g., "Uploading chunks", and the number of chunks
    /// to upload.
    ///
    /// The text output by `println` on the bar will appear above the bar, so it functions as a
    /// good title for what the bar is doing.
    pub fn init_new_safe_task(&self, task_title: &str, task_len: u64) {
        self.safe_task_progress_bar.println(task_title);
        self.safe_task_progress_bar.set_length(task_len);
        self.safe_task_progress_bar.set_position(0);
    }

    /// Increment the safe progress bar through its current task
    pub fn inc_safe_task(&self, delta: u64) {
        self.safe_task_progress_bar.inc(delta);
    }

    /// Set the text for the bar related to feedback from work performed within `safe`
    pub fn set_safe_text(&self, msg: &str) {
        self.safe_text_bar.set_message(msg.to_string());
    }

    /// Set the text for the bar related to feedback from the client
    pub fn set_client_feedback(&self, msg: &str) {
        self.client_text_bar.set_message(msg.to_string());
    }

    /// Stop using the bars after you've done everything requiring feedback.
    ///
    /// You can then resume use of simple `println` macros.
    pub fn clear(&self) -> Result<()> {
        self.multi_bar.clear()?;
        Ok(())
    }
}
