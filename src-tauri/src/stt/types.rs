use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
	User,
	AI,
	VAD,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Task {
	GiveHint,
	AskQuestion,
	None,
}

impl fmt::Display for Task {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		let s = match self {
			Task::AskQuestion => "(ask)",
			Task::GiveHint => "(hint)",
			Task::None => "",
		};
		write!(f, "{}", s)
	}
}

#[derive(Debug, Clone)]
pub struct Dialogue {
	pub role: Role,
	pub content: String,
	pub task: Task,
}

impl Dialogue {
	pub fn new(role: Role, content: String, task: Task) -> Self {
		Self { role, content, task }
	}
}

impl fmt::Display for Dialogue {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self.role {
			Role::User => write!(f, "User: {}\n", self.content),
			Role::AI => write!(f, "AI{}: {}\n", self.task, self.content),
			Role::VAD => write!(f, "[VAD {}]\n", self.content),
		}
	}
}

pub struct Conversation(pub Vec<Dialogue>);

impl fmt::Display for Conversation {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		for dialogue in &self.0 {
			write!(f, "{}", dialogue)?;
		}
		Ok(())
	}
}

impl std::ops::Deref for Conversation {
	type Target = Vec<Dialogue>;

	fn deref(&self) -> &Self::Target {
		&self.0
	}
}
