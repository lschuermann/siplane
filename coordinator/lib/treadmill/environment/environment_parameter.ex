defmodule Treadmill.Environment.Parameter do
  use Ecto.Schema

  @primary_key false
  @foreign_key_type :binary_id
  schema "environment_parameters" do
    field :key, :string
    field :value, :string
    field :secret, :boolean

    belongs_to :environment, Treadmill.Environment

    timestamps(type: :utc_datetime)
  end
end
