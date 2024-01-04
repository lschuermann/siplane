{ python3Packages, fetchFromGitHub }:

python3Packages.buildPythonPackage rec {
  pname = "mkosi";
  version = "git-${builtins.substring 0 14 rev}";
  rev = "b71deeba585b84f2eb362b70a45cbd22fd756a4d";

  src = fetchFromGitHub {
    owner = "systemd";
    repo = pname;
    rev = rev;
    sha256 = "sha256-KcW3r6NeoVe4J7cO52HoF3WTVhOnPyS72m1HsdWZqwg=";
  };

  patches = [
    ./0001-Nix-compatibility.patch
  ];

  format = "pyproject";

  nativeBuildInputs = with python3Packages; [
    setuptools setuptools-scm
  ];
}
