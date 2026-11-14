// Read native process ledgers without launching or signalling a process.
#include <errno.h>
#include <limits.h>
#include <libproc.h>
#include <stdio.h>
#include <stdlib.h>
#include <sys/resource.h>

int main(int argc, char **argv) {
  if (argc < 2) {
    fputs("usage: macos_usage PID [PID ...]\n", stderr);
    return 2;
  }
  // Validate the whole batch before writing JSON.
  for (int i = 1; i < argc; i++) {
    char *end;
    errno = 0;
    long pid = strtol(argv[i], &end, 10);
    if (errno || end == argv[i] || *end || pid <= 0 || pid > INT_MAX) {
      fputs("expected a positive PID within int range\n", stderr);
      return 2;
    }
  }
  int failed = 0;
  putchar('[');
  for (int i = 1; i < argc; i++) {
    int pid = (int)strtol(argv[i], NULL, 10);
    struct rusage_info_v4 usage = {0};
    int status = proc_pid_rusage(pid, RUSAGE_INFO_V4, (rusage_info_t *)&usage);
    int error = errno;
    if (i > 1) putchar(',');
    printf("{\"pid\":%d", pid);
    if (status) {
      printf(",\"error\":%d", error);
      failed = 1;
    } else {
      printf(",\"rssBytes\":%llu,\"footprintBytes\":%llu,\"startAbs\":%llu",
             (unsigned long long)usage.ri_resident_size,
             (unsigned long long)usage.ri_phys_footprint,
             (unsigned long long)usage.ri_proc_start_abstime);
    }
    putchar('}');
  }
  puts("]");
  return failed;
}
