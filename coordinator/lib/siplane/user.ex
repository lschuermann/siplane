defmodule Siplane.User do
  use Ecto.Schema
  import Ecto.Changeset

  @primary_key {:id, :binary_id, autogenerate: true}
  @foreign_key_type :binary_id
  schema "users" do
    field :name, :string
    field :email, :string

    timestamps(type: :utc_datetime)

    has_many :user_providers, Siplane.User.UserProvider
  end

  @doc false
  def changeset(user, attrs) do
    user
    |> cast(attrs, [:name, :email])
    |> validate_required([:name, :email])
    # |> unique_constraint(:email)
  end
end

defmodule Siplane.User.UserProvider do
  use Ecto.Schema
  import Ecto.Changeset

  @primary_key false
  @foreign_key_type :binary_id
  schema "user_providers" do
    field :provider, :string
    field :token, :string

    belongs_to :user, Siplane.User

    timestamps(type: :utc_datetime)
  end

  @doc false
  def changeset(user_provider, attrs) do
    user_provider
    |> cast(attrs, [:user_id, :provider, :token])
    |> validate_required([:user_id, :provider, :token])
    |> unique_constraint([:user_id, :provider])
  end

  @doc false
  def changeset_with_user(user_provider, attrs) do
    user_provider
    |> cast(attrs, [:provider, :token])
    |> cast_assoc(:user, with: &Siplane.User.changeset/2)
    |> validate_required([:user, :provider, :token])
    |> unique_constraint([:user, :provider])
  end
end
