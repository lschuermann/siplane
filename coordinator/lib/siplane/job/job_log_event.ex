defmodule Siplane.Job.LogEvent do
  use Ecto.Schema
  # import Ecto.Changeset

  @primary_key false
  @foreign_key_type :binary_id
  schema "log_event_jobs" do
    belongs_to :log_event, Siplane.Log
    belongs_to :job, Siplane.Job

    field :public, :boolean
    field :creator_visible, :boolean
  end
end
