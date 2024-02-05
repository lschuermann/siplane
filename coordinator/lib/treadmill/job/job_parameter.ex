defmodule Treadmill.Job.Parameter do
  use Ecto.Schema

  @primary_key false
  @foreign_key_type :binary_id
  schema "job_parameters" do
    field :key, :string
    field :value, :string
    field :secret, :boolean

    belongs_to :job, Treadmill.Job

    timestamps(type: :utc_datetime)
  end
end
