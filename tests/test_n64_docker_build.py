import json
import os
import pathlib
import subprocess
import sys
import tempfile
import unittest


ROOT = pathlib.Path(__file__).resolve().parents[1]
SOURCE_REV = "a" * 40
IMAGE_ID = "sha256:" + "b" * 64


class N64DockerBuildTests(unittest.TestCase):
  def setUp(self) -> None:
    self.temp = tempfile.TemporaryDirectory()
    self.addCleanup(self.temp.cleanup)
    self.directory = pathlib.Path(self.temp.name)
    self.cache = self.directory / "cache"
    self.log = self.directory / "calls.jsonl"
    tools = self.directory / "bin"
    tools.mkdir()
    fake = f"""#!{sys.executable}
import json
import os
import pathlib
import shutil
import sys

tool = pathlib.Path(sys.argv[0]).name
args = sys.argv[1:]
with open(os.environ["CALL_LOG"], "a", encoding="utf-8") as log:
  log.write(json.dumps([tool, *args]) + "\\n")
if tool == "git":
  if args[0] == "init":
    pathlib.Path(args[1], ".git").mkdir(parents=True)
  elif args[0] == "clone":
    pathlib.Path(args[-1], ".git").mkdir(parents=True)
  elif "rev-parse" in args:
    print(os.environ["SOURCE_REV"])
  elif "status" in args and os.environ.get("DIRTY_SOURCE"):
    print(" M local-change.c")
elif tool == "make":
  if args == ["clean"]:
    shutil.rmtree("build", ignore_errors=True)
  elif "-C" in args:
    if "install" in args and "tools-install" in args:
      if os.environ.get("SDK_FAILURE"):
        sys.exit(1)
      sdk = pathlib.Path(os.environ["N64_INST"])
      for name in ("include/n64.mk", "mips64-elf/lib/libdragon.a", "bin/n64tool"):
        output = sdk / name
        output.parent.mkdir(parents=True, exist_ok=True)
        output.touch()
        output.chmod(0o755)
  else:
    pathlib.Path("build").mkdir(exist_ok=True)
    if os.environ.get("ROM_FAILURE"):
      sys.exit(1)
    pathlib.Path("build/main.o").write_text(os.environ["LIBDRAGON_BUILD_ID"])
elif args[0] == "build" and os.environ.get("BUILD_FAILURE"):
  sys.exit(1)
elif args[:2] == ["image", "inspect"]:
  if "--format" not in args and os.environ.get("MISSING_IMAGE"):
    sys.exit(1)
  print(os.environ["IMAGE_ID"])
"""
    for tool in ("git", "docker", "make"):
      executable = tools / tool
      executable.write_text(fake, encoding="utf-8")
      executable.chmod(0o755)
    self.env = {key: value for key, value in os.environ.items()
                if not key.startswith("LIBDRAGON_")}
    self.env.update({
      "PATH": str(tools) + os.pathsep + os.environ["PATH"],
      "CALL_LOG": str(self.log),
      "SOURCE_REV": SOURCE_REV,
      "IMAGE_ID": IMAGE_ID,
      "LIBDRAGON_DOCKER_CACHE": str(self.cache),
    })

  def run_build(self, *args: str) -> subprocess.CompletedProcess:
    return subprocess.run(
      ["bash", str(ROOT / "scripts" / "n64-docker-build.sh"), *args],
      env=self.env, capture_output=True, text=True, check=False,
    )

  def calls(self) -> list:
    if not self.log.exists():
      return []
    return [json.loads(line) for line in self.log.read_text().splitlines()]

  def test_default_builds_image_and_pins_source_and_run_identity(self) -> None:
    result = self.run_build("-j2", "N64_ROM_TITLE=chip viz")
    self.assertEqual(result.returncode, 0, result.stderr)
    calls = self.calls()
    self.assertTrue(any(call[:2] == ["docker", "build"] for call in calls))
    fetch = next(call for call in calls if call[0] == "git" and "fetch" in call)
    self.assertRegex(fetch[-1], r"^[0-9a-f]{40}$")
    run = next(call for call in calls if call[:2] == ["docker", "run"])
    self.assertIn(f"LIBDRAGON_BUILD_ID={SOURCE_REV} {IMAGE_ID}", run)
    self.assertEqual(run[run.index("sh") - 1], IMAGE_ID)
    self.assertEqual(run[-3:], ["sh", "-j2", "N64_ROM_TITLE=chip viz"])

  def test_custom_image_and_source_overrides_remain_available(self) -> None:
    self.env.update({
      "LIBDRAGON_DOCKER_IMAGE": "example/custom:toolchain",
      "LIBDRAGON_BRANCH": "custom-sdk",
      "MISSING_IMAGE": "1",
    })
    result = self.run_build()
    self.assertEqual(result.returncode, 0, result.stderr)
    calls = self.calls()
    self.assertFalse(any(call[:2] == ["docker", "build"] for call in calls))
    self.assertIn(
      ["docker", "pull", "--platform", "linux/amd64", "example/custom:toolchain"],
      calls,
    )
    fetch = next(call for call in calls if call[0] == "git" and "fetch" in call)
    self.assertEqual(fetch[-1], "custom-sdk")

  def test_image_build_failure_prevents_rom_build(self) -> None:
    self.env["BUILD_FAILURE"] = "1"
    result = self.run_build()
    self.assertNotEqual(result.returncode, 0)
    self.assertFalse(any(call[:2] == ["docker", "run"] for call in self.calls()))

  def test_existing_non_git_cache_is_not_deleted(self) -> None:
    source = self.cache / "source"
    source.mkdir(parents=True)
    sentinel = source / "keep.txt"
    sentinel.write_text("keep", encoding="utf-8")
    result = self.run_build()
    self.assertNotEqual(result.returncode, 0)
    self.assertIn("Refusing to overwrite", result.stderr)
    self.assertEqual(sentinel.read_text(), "keep")
    self.assertEqual(self.calls(), [])

  def test_modified_source_cache_is_not_rebuilt(self) -> None:
    (self.cache / "source" / ".git").mkdir(parents=True)
    self.env["DIRTY_SOURCE"] = "1"
    result = self.run_build()
    self.assertNotEqual(result.returncode, 0)
    self.assertIn("modified source cache", result.stderr)
    self.assertFalse(any("fetch" in call or call[0] == "docker" for call in self.calls()))

  def test_different_images_get_separate_source_and_sdk_caches(self) -> None:
    first = self.run_build()
    self.assertEqual(first.returncode, 0, first.stderr)
    self.env["IMAGE_ID"] = "sha256:" + "c" * 64
    second = self.run_build()
    self.assertEqual(second.returncode, 0, second.stderr)
    runs = [call for call in self.calls() if call[:2] == ["docker", "run"]]
    mounts = [next(arg for arg in call if arg.endswith(":/cache")) for call in runs]
    self.assertNotEqual(mounts[0], mounts[1])
    clones = [call for call in self.calls() if call[:2] == ["git", "clone"]]
    self.assertEqual(len(clones), 2)
    self.assertNotEqual(clones[0][-1], clones[1][-1])

  def container_script(self) -> str:
    result = self.run_build()
    self.assertEqual(result.returncode, 0, result.stderr)
    run = next(call for call in self.calls() if call[:2] == ["docker", "run"])
    return run[run.index("-lc") + 1]

  def run_container(self, script: str, identity: str) -> subprocess.CompletedProcess:
    cache = self.directory / "container"
    (cache / "sdk").mkdir(parents=True, exist_ok=True)
    workspace = self.directory / "rom"
    workspace.mkdir(exist_ok=True)
    env = self.env | {
      "N64_INST": str(cache / "sdk"),
      "LIBDRAGON_BUILD_ID": identity,
    }
    return subprocess.run(
      ["sh", "-c", script.replace("/cache", str(cache)), "sh"],
      cwd=workspace, env=env, capture_output=True, text=True, check=False,
    )

  def test_populated_sdk_switch_still_cleans_rom_objects(self) -> None:
    script = self.container_script()
    first = self.run_container(script, "toolchain-A")
    self.assertEqual(first.returncode, 0, first.stderr)
    stamp = self.directory / "container/sdk/.libdragon-build"
    stamp.write_text("toolchain-B\n")
    before = len(self.calls())
    second = self.run_container(script, "toolchain-B")
    self.assertEqual(second.returncode, 0, second.stderr)
    calls = self.calls()[before:]
    self.assertIn(["make", "clean"], calls)
    self.assertFalse(any("-C" in call for call in calls))
    self.assertEqual((self.directory / "rom/build/main.o").read_text(), "toolchain-B")
    before = len(self.calls())
    repeat = self.run_container(script, "toolchain-B")
    self.assertEqual(repeat.returncode, 0, repeat.stderr)
    self.assertEqual(self.calls()[before:], [["make"]])

  def test_failed_sdk_build_is_not_stamped_and_can_retry(self) -> None:
    script = self.container_script()
    self.env["SDK_FAILURE"] = "1"
    failed = self.run_container(script, "toolchain-A")
    self.assertNotEqual(failed.returncode, 0)
    sdk_stamp = self.directory / "container/sdk/.libdragon-build"
    rom_stamp = self.directory / "rom/build/.chipviz-toolchain"
    self.assertFalse(sdk_stamp.exists())
    self.assertFalse(rom_stamp.exists())
    del self.env["SDK_FAILURE"]
    retry = self.run_container(script, "toolchain-A")
    self.assertEqual(retry.returncode, 0, retry.stderr)
    self.assertEqual(sdk_stamp.read_text().strip(), "toolchain-A")
    self.assertEqual(rom_stamp.read_text().strip(), "toolchain-A")

  def test_failed_rom_build_does_not_stamp_the_new_toolchain(self) -> None:
    script = self.container_script()
    first = self.run_container(script, "toolchain-A")
    self.assertEqual(first.returncode, 0, first.stderr)
    self.env["ROM_FAILURE"] = "1"
    failed = self.run_container(script, "toolchain-B")
    self.assertNotEqual(failed.returncode, 0)
    self.assertFalse((self.directory / "rom/build/.chipviz-toolchain").exists())
    del self.env["ROM_FAILURE"]
    retry = self.run_container(script, "toolchain-B")
    self.assertEqual(retry.returncode, 0, retry.stderr)
    self.assertEqual(
      (self.directory / "rom/build/.chipviz-toolchain").read_text().strip(),
      "toolchain-B",
    )
