defmodule Siplane.Environment do
  use Ecto.Schema
  import Ecto.Changeset

  def log_event(_event) do
    # nada
  end

  @primary_key {:id, :binary_id, autogenerate: true}
  @foreign_key_type :binary_id
  schema "environments" do
    field :label, :string

    timestamps(type: :utc_datetime)

    many_to_many :boards, Siplane.Board,
      join_through: Siplane.BoardEnvironment
    has_many :jobs, Siplane.Job
  end

  @doc false
  def changeset(user, attrs) do
    # TODO: boards foreign key constraint?
    user
    |> cast(attrs, [:label])
    |> validate_required([:label])
  end
end
